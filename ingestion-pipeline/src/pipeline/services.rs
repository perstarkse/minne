use std::{
    ops::Range,
    sync::{Arc, OnceLock},
};

use anyhow::Context;
use async_openai::types::{
    ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
    CreateChatCompletionRequest, CreateChatCompletionRequestArgs, ResponseFormat,
    ResponseFormatJsonSchema,
};
use async_trait::async_trait;
use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        store::StorageManager,
        types::{
            ingestion_payload::IngestionPayload, knowledge_relationship::KnowledgeRelationship,
            system_settings::SystemSettings, text_chunk::TextChunk, text_content::TextContent,
            StoredObject,
        },
    },
    utils::{config::AppConfig, embedding::EmbeddingProvider},
};
use retrieval_pipeline::{reranking::RerankerPool, retrieved_entities_to_json, RetrievedEntity};
use text_splitter::{ChunkCapacity, ChunkConfig, TextSplitter};

use super::{enrichment_result::LLMEnrichmentResult, preparation::to_text_content};
use crate::pipeline::context::{EmbeddedKnowledgeEntity, EmbeddedTextChunk};
use crate::utils::llm_instructions::{
    get_ingress_analysis_schema, INGRESS_ANALYSIS_SYSTEM_MESSAGE,
};

const EMBEDDING_QUERY_CHAR_LIMIT: usize = 12_000;
#[async_trait]
pub trait PipelineServices: Send + Sync {
    async fn prepare_text_content(
        &self,
        payload: IngestionPayload,
    ) -> Result<TextContent, AppError>;

    async fn retrieve_similar_entities(
        &self,
        content: &TextContent,
    ) -> Result<Vec<RetrievedEntity>, AppError>;

    async fn run_enrichment(
        &self,
        content: &TextContent,
        similar_entities: &[RetrievedEntity],
    ) -> Result<LLMEnrichmentResult, AppError>;

    async fn convert_analysis(
        &self,
        content: &TextContent,
        analysis: &LLMEnrichmentResult,
        entity_concurrency: usize,
    ) -> Result<(Vec<EmbeddedKnowledgeEntity>, Vec<KnowledgeRelationship>), AppError>;

    async fn prepare_chunks(
        &self,
        content: &TextContent,
        token_range: Range<usize>,
        overlap_tokens: usize,
    ) -> Result<Vec<EmbeddedTextChunk>, AppError>;
}

pub struct DefaultPipelineServices {
    db: Arc<SurrealDbClient>,
    openai_client: Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
    config: AppConfig,
    reranker_pool: Option<Arc<RerankerPool>>,
    storage: StorageManager,
    embedding_provider: Arc<EmbeddingProvider>,
}

impl DefaultPipelineServices {
    pub fn new(
        db: Arc<SurrealDbClient>,
        openai_client: Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
        config: AppConfig,
        reranker_pool: Option<Arc<RerankerPool>>,
        storage: StorageManager,
        embedding_provider: Arc<EmbeddingProvider>,
    ) -> Self {
        Self {
            db,
            openai_client,
            config,
            reranker_pool,
            storage,
            embedding_provider,
        }
    }

    async fn prepare_llm_request(
        &self,
        category: &str,
        context: Option<&str>,
        text: &str,
        similar_entities: &[RetrievedEntity],
    ) -> Result<CreateChatCompletionRequest, AppError> {
        let settings = SystemSettings::get_current(&self.db).await?;

        let entities_json = retrieved_entities_to_json(similar_entities);

        let user_message = format!(
            "Category:\n{category}\ncontext:\n{context:?}\nContent:\n{text}\nExisting KnowledgeEntities in database:\n{entities_json}"
        );

        let response_format = ResponseFormat::JsonSchema {
            json_schema: ResponseFormatJsonSchema {
                description: Some("Structured analysis of the submitted content".into()),
                name: "content_analysis".into(),
                schema: Some(get_ingress_analysis_schema()),
                strict: Some(true),
            },
        };

        let request = CreateChatCompletionRequestArgs::default()
            .model(&settings.processing_model)
            .messages([
                ChatCompletionRequestSystemMessage::from(INGRESS_ANALYSIS_SYSTEM_MESSAGE).into(),
                ChatCompletionRequestUserMessage::from(user_message).into(),
            ])
            .response_format(response_format)
            .build()?;

        Ok(request)
    }

    async fn perform_analysis(
        &self,
        request: CreateChatCompletionRequest,
    ) -> Result<LLMEnrichmentResult, AppError> {
        let response = self.openai_client.chat().create(request).await?;

        let content = response
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_ref())
            .ok_or(AppError::LLMParsing(
                "No content found in LLM response".into(),
            ))?;

        serde_json::from_str::<LLMEnrichmentResult>(content).map_err(|e| {
            AppError::LLMParsing(format!("Failed to parse LLM response into analysis: {e}"))
        })
    }
}

#[async_trait]
impl PipelineServices for DefaultPipelineServices {
    async fn prepare_text_content(
        &self,
        payload: IngestionPayload,
    ) -> Result<TextContent, AppError> {
        to_text_content(
            payload,
            &self.db,
            &self.config,
            &self.openai_client,
            &self.storage,
        )
        .await
    }

    async fn retrieve_similar_entities(
        &self,
        content: &TextContent,
    ) -> Result<Vec<RetrievedEntity>, AppError> {
        let truncated_body = truncate_for_embedding(&content.text, EMBEDDING_QUERY_CHAR_LIMIT);
        let input_text = format!(
            "content: {}\n[truncated={}], category: {}, user_context: {:?}",
            truncated_body,
            truncated_body.len() < content.text.len(),
            content.category,
            content.context
        );

        let rerank_lease = match &self.reranker_pool {
            Some(pool) => Some(pool.checkout().await),
            None => None,
        };

        let config = retrieval_pipeline::RetrievalConfig::for_search(
            retrieval_pipeline::SearchTarget::EntitiesOnly,
        );
        match retrieval_pipeline::retrieve_entities(
            &self.db,
            &self.openai_client,
            Some(&*self.embedding_provider),
            &input_text,
            &content.user_id,
            config,
            rerank_lease,
        )
        .await
        {
            Ok(retrieval_pipeline::StrategyOutput::Entities(entities)) => Ok(entities),
            Ok(retrieval_pipeline::StrategyOutput::Search(search)) => {
                let chunk_count = search.chunks.len();
                let entities = search.entities;
                tracing::debug!(
                    chunk_count,
                    entity_count = entities.len(),
                    "ingestion search results returned entities"
                );
                Ok(entities)
            }
            Ok(retrieval_pipeline::StrategyOutput::Chunks(_)) => Err(AppError::InternalError(
                "Ingestion retrieval should return entities".into(),
            )),
            Err(e) => Err(e),
        }
    }

    async fn run_enrichment(
        &self,
        content: &TextContent,
        similar_entities: &[RetrievedEntity],
    ) -> Result<LLMEnrichmentResult, AppError> {
        let request = self
            .prepare_llm_request(
                &content.category,
                content.context.as_deref(),
                &content.text,
                similar_entities,
            )
            .await?;
        self.perform_analysis(request).await
    }

    async fn convert_analysis(
        &self,
        content: &TextContent,
        analysis: &LLMEnrichmentResult,
        entity_concurrency: usize,
    ) -> Result<(Vec<EmbeddedKnowledgeEntity>, Vec<KnowledgeRelationship>), AppError> {
        analysis
            .to_database_entities(
                content.get_id(),
                &content.user_id,
                &self.openai_client,
                &self.db,
                entity_concurrency,
                Some(&*self.embedding_provider),
            )
            .await
    }

    async fn prepare_chunks(
        &self,
        content: &TextContent,
        token_range: Range<usize>,
        overlap_tokens: usize,
    ) -> Result<Vec<EmbeddedTextChunk>, AppError> {
        let chunk_candidates = prepare_chunks(
            &content.text,
            token_range.start,
            token_range.end,
            overlap_tokens,
        )?;

        let mut chunks = Vec::with_capacity(chunk_candidates.len());
        for chunk_text in chunk_candidates {
            let embedding = self
                .embedding_provider
                .embed(&chunk_text)
                .await
                .context("generating FastEmbed embedding for chunk")?;
            let chunk_struct = TextChunk::new(
                content.get_id().to_string(),
                chunk_text,
                content.user_id.clone(),
            );
            chunks.push(EmbeddedTextChunk {
                chunk: chunk_struct,
                embedding,
            });
        }
        Ok(chunks)
    }
}

fn prepare_chunks(
    text: &str,
    min_tokens: usize,
    max_tokens: usize,
    overlap_tokens: usize,
) -> Result<Vec<String>, AppError> {
    if min_tokens == 0 || max_tokens == 0 || min_tokens > max_tokens {
        return Err(AppError::Validation(
            "invalid chunk token bounds; ensure 0 < min <= max".into(),
        ));
    }

    if overlap_tokens >= min_tokens {
        return Err(AppError::Validation(format!(
            "chunk_min_tokens must be greater than the configured overlap of {overlap_tokens}"
        )));
    }

    let tokenizer = get_tokenizer()?;

    let chunk_capacity = ChunkCapacity::new(min_tokens)
        .with_max(max_tokens)
        .map_err(|e| AppError::Validation(format!("invalid chunk token bounds: {e}")))?;
    let chunk_config = ChunkConfig::new(chunk_capacity)
        .with_overlap(overlap_tokens)
        .map_err(|e| AppError::Validation(format!("invalid chunk overlap: {e}")))?
        .with_sizer(tokenizer);
    let splitter = TextSplitter::new(chunk_config);

    let mut chunks: Vec<String> = splitter.chunks(text).map(str::to_owned).collect();

    if chunks.is_empty() {
        chunks.push(String::new());
    }

    Ok(chunks)
}

fn get_tokenizer() -> Result<&'static tokenizers::Tokenizer, AppError> {
    static TOKENIZER: OnceLock<Result<tokenizers::Tokenizer, String>> = OnceLock::new();

    match TOKENIZER.get_or_init(|| {
        tokenizers::Tokenizer::from_pretrained("bert-base-cased", None)
            .map_err(|e| format!("failed to initialize tokenizer: {e}"))
    }) {
        Ok(tokenizer) => Ok(tokenizer),
        Err(err) => Err(AppError::InternalError(err.clone())),
    }
}

fn truncate_for_embedding(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let mut truncated = String::with_capacity(max_chars.saturating_add(3));
    for (idx, ch) in text.chars().enumerate() {
        if idx >= max_chars {
            break;
        }
        truncated.push(ch);
    }
    truncated.push('…');
    truncated
}
