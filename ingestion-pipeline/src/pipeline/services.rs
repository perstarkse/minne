use std::{ops::Range, sync::Arc};

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
            ingestion_payload::IngestionPayload, knowledge_entity::KnowledgeEntity,
            knowledge_relationship::KnowledgeRelationship, system_settings::SystemSettings,
            text_chunk::TextChunk, text_content::TextContent,
        },
    },
    utils::{config::AppConfig, embedding::generate_embedding},
};
use retrieval_pipeline::{
    reranking::RerankerPool, retrieve_entities, retrieved_entities_to_json, RetrievalConfig,
    RetrievalStrategy, RetrievedEntity, StrategyOutput,
};
use text_splitter::TextSplitter;

use super::{enrichment_result::LLMEnrichmentResult, preparation::to_text_content};
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
    ) -> Result<(Vec<KnowledgeEntity>, Vec<KnowledgeRelationship>), AppError>;

    async fn prepare_chunks(
        &self,
        content: &TextContent,
        range: Range<usize>,
    ) -> Result<Vec<TextChunk>, AppError>;
}

pub struct DefaultPipelineServices {
    db: Arc<SurrealDbClient>,
    openai_client: Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
    config: AppConfig,
    reranker_pool: Option<Arc<RerankerPool>>,
    storage: StorageManager,
}

impl DefaultPipelineServices {
    pub fn new(
        db: Arc<SurrealDbClient>,
        openai_client: Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
        config: AppConfig,
        reranker_pool: Option<Arc<RerankerPool>>,
        storage: StorageManager,
    ) -> Self {
        Self {
            db,
            openai_client,
            config,
            reranker_pool,
            storage,
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

    fn configured_strategy(&self) -> RetrievalStrategy {
        self.config
            .retrieval_strategy
            .as_deref()
            .and_then(|value| value.parse().ok())
            .unwrap_or(RetrievalStrategy::Initial)
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

        let mut config = RetrievalConfig::default();
        config.strategy = self.configured_strategy();
        match retrieve_entities(
            &self.db,
            &self.openai_client,
            &input_text,
            &content.user_id,
            config,
            rerank_lease,
        )
        .await
        {
            Ok(StrategyOutput::Entities(entities)) => Ok(entities),
            Ok(StrategyOutput::Chunks(_)) => Err(AppError::InternalError(
                "Chunk-only retrieval is not supported in ingestion".into(),
            )),
            Err(err) => Err(err),
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
    ) -> Result<(Vec<KnowledgeEntity>, Vec<KnowledgeRelationship>), AppError> {
        analysis
            .to_database_entities(
                &content.id,
                &content.user_id,
                &self.openai_client,
                &self.db,
                entity_concurrency,
            )
            .await
    }

    async fn prepare_chunks(
        &self,
        content: &TextContent,
        range: Range<usize>,
    ) -> Result<Vec<TextChunk>, AppError> {
        let splitter = TextSplitter::new(range.clone());
        let chunk_texts: Vec<String> = splitter
            .chunks(&content.text)
            .map(|chunk| chunk.to_string())
            .collect();

        let mut chunks = Vec::with_capacity(chunk_texts.len());
        for chunk in chunk_texts {
            let embedding = generate_embedding(&self.openai_client, &chunk, &self.db).await?;
            chunks.push(TextChunk::new(
                content.id.clone(),
                chunk,
                embedding,
                content.user_id.clone(),
            ));
        }
        Ok(chunks)
    }
}

fn truncate_for_embedding(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let mut truncated = String::with_capacity(max_chars + 3);
    for (idx, ch) in text.chars().enumerate() {
        if idx >= max_chars {
            break;
        }
        truncated.push(ch);
    }
    truncated.push_str("…");
    truncated
}
