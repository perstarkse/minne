use std::{
    ops::Range,
    sync::{Arc, OnceLock},
};

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
use crate::utils::llm_instructions::get_ingress_analysis_schema;

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
    embedding_query_char_limit: usize,
}

impl DefaultPipelineServices {
    pub fn new(
        db: Arc<SurrealDbClient>,
        openai_client: Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
        config: AppConfig,
        reranker_pool: Option<Arc<RerankerPool>>,
        storage: StorageManager,
        embedding_provider: Arc<EmbeddingProvider>,
        embedding_query_char_limit: usize,
    ) -> Self {
        Self {
            db,
            openai_client,
            config,
            reranker_pool,
            storage,
            embedding_provider,
            embedding_query_char_limit,
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
                ChatCompletionRequestSystemMessage::from(settings.ingestion_system_prompt.as_str())
                    .into(),
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
        let truncated_body = truncate_for_embedding(&content.text, self.embedding_query_char_limit);
        let input_text = format!(
            "content: {}\n[truncated={}], category: {}, user_context: {:?}",
            truncated_body,
            truncated_body.len() < content.text.len(),
            content.category,
            content.context
        );

        let rerank_lease = match &self.reranker_pool {
            Some(pool) => pool.checkout().await,
            None => None,
        };

        let config = retrieval_pipeline::RetrievalConfig::with_entities();
        match retrieval_pipeline::retrieve(
            &self.db,
            &self.embedding_provider,
            &input_text,
            &content.user_id,
            config,
            rerank_lease,
        )
        .await
        {
            Ok(retrieval_pipeline::RetrievalOutput::WithEntities { chunks, entities }) => {
                tracing::debug!(
                    chunk_count = chunks.len(),
                    entity_count = entities.len(),
                    "ingestion retrieval resolved entities from chunks"
                );
                Ok(entities)
            }
            Ok(retrieval_pipeline::RetrievalOutput::Chunks(_)) => Err(AppError::InternalError(
                "Ingestion retrieval should resolve entities".into(),
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
                content.id(),
                &content.user_id,
                entity_concurrency,
                &self.embedding_provider,
            )
            .await
    }

    async fn prepare_chunks(
        &self,
        content: &TextContent,
        token_range: Range<usize>,
        overlap_tokens: usize,
    ) -> Result<Vec<EmbeddedTextChunk>, AppError> {
        let chunk_candidates = split_text_into_chunks(
            &content.text,
            token_range.start,
            token_range.end,
            overlap_tokens,
        )?;

        if chunk_candidates.is_empty() {
            return Ok(Vec::new());
        }

        // Embed all chunks of this document in one batch: a single lock acquisition and one
        // blocking hop, letting the backend batch the inference internally.
        let embeddings = self
            .embedding_provider
            .embed_batch(chunk_candidates.clone())
            .await
            .map_err(|e| {
                AppError::InternalError(format!("FastEmbed embedding for chunks failed: {e}"))
            })?;

        if embeddings.len() != chunk_candidates.len() {
            return Err(AppError::InternalError(format!(
                "embedding batch returned {} vectors for {} chunks",
                embeddings.len(),
                chunk_candidates.len()
            )));
        }

        let mut chunks = Vec::with_capacity(chunk_candidates.len());
        for (chunk_text, embedding) in chunk_candidates.into_iter().zip(embeddings) {
            let chunk_struct = TextChunk::new(
                content.id().to_string(),
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

fn split_text_into_chunks(
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::Context;
    use async_openai::{config::OpenAIConfig, types::ChatCompletionRequestMessage, Client};
    use common::{
        storage::{
            db::SurrealDbClient, store::StorageManager, types::system_settings::SystemSettingsPatch,
        },
        utils::{
            config::{AppConfig, StorageKind},
            embedding::EmbeddingProvider,
        },
    };
    use uuid::Uuid;

    use super::DefaultPipelineServices;
    use crate::pipeline::IngestionTuning;
    use common::error::AppError;

    fn system_prompt_from_request(
        request: &async_openai::types::CreateChatCompletionRequest,
    ) -> anyhow::Result<String> {
        let Some(ChatCompletionRequestMessage::System(system)) = request.messages.first() else {
            anyhow::bail!("expected first message to be system");
        };
        let async_openai::types::ChatCompletionRequestSystemMessageContent::Text(text) =
            &system.content
        else {
            anyhow::bail!("unexpected system message content: {:?}", system.content);
        };
        Ok(text.clone())
    }

    #[tokio::test]
    async fn prepare_llm_request_uses_ingestion_prompt_from_system_settings() -> anyhow::Result<()>
    {
        const SENTINEL: &str = "ingestion-prompt-sentinel-from-db";

        let db = Arc::new(
            SurrealDbClient::memory("test_ns", &Uuid::new_v4().to_string())
                .await
                .context("start in-memory db")?,
        );
        db.apply_migrations().await.context("apply migrations")?;
        SystemSettingsPatch {
            ingestion_system_prompt: Some(SENTINEL.to_string()),
            ..Default::default()
        }
        .apply(&db)
        .await
        .context("patch ingestion prompt")?;

        let config = AppConfig {
            storage: StorageKind::Memory,
            ..Default::default()
        };
        let storage = StorageManager::new(&config)
            .await
            .context("storage manager")?;
        let openai_client = Arc::new(Client::with_config(OpenAIConfig::default()));
        let embedding_provider = Arc::new(EmbeddingProvider::new_hashed(384)?);

        let services = DefaultPipelineServices::new(
            db,
            openai_client,
            config,
            None,
            storage,
            embedding_provider,
            IngestionTuning::default().embedding_query_char_limit,
        );

        let request = services
            .prepare_llm_request("notes", None, "hello world", &[])
            .await
            .context("prepare llm request")?;

        assert_eq!(system_prompt_from_request(&request)?, SENTINEL);
        Ok(())
    }

    #[test]
    fn split_text_into_chunks_rejects_zero_bounds() {
        assert!(matches!(
            super::split_text_into_chunks("text", 0, 10, 0),
            Err(AppError::Validation(_))
        ));
        assert!(matches!(
            super::split_text_into_chunks("text", 4, 0, 0),
            Err(AppError::Validation(_))
        ));
    }

    #[test]
    fn split_text_into_chunks_rejects_min_greater_than_max() {
        assert!(matches!(
            super::split_text_into_chunks("text", 10, 4, 0),
            Err(AppError::Validation(_))
        ));
    }

    #[test]
    fn split_text_into_chunks_rejects_overlap_at_or_above_min() {
        assert!(matches!(
            super::split_text_into_chunks("text", 4, 10, 4),
            Err(AppError::Validation(_))
        ));
        assert!(matches!(
            super::split_text_into_chunks("text", 4, 10, 5),
            Err(AppError::Validation(_))
        ));
    }

    #[test]
    fn truncate_for_embedding_returns_short_text_unchanged() {
        assert_eq!(super::truncate_for_embedding("hello", 10), "hello");
        // Exactly at the limit is left untouched (no ellipsis appended).
        assert_eq!(super::truncate_for_embedding("hello", 5), "hello");
    }

    #[test]
    fn truncate_for_embedding_appends_ellipsis_when_over_limit() {
        assert_eq!(super::truncate_for_embedding("hello world", 5), "hello…");
    }

    #[test]
    fn truncate_for_embedding_respects_char_boundaries() {
        // Multibyte characters must not be split mid-byte.
        let truncated = super::truncate_for_embedding("héllo wörld", 4);
        assert_eq!(truncated, "héll…");
        assert_eq!(truncated.chars().count(), 5);
    }
}
