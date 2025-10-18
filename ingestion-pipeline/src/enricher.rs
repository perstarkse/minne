use std::sync::Arc;

use async_openai::types::{
    ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
    CreateChatCompletionRequest, CreateChatCompletionRequestArgs, ResponseFormat,
    ResponseFormatJsonSchema,
};
use common::{
    error::AppError,
    storage::{db::SurrealDbClient, types::system_settings::SystemSettings},
};
use composite_retrieval::{retrieve_entities, retrieved_entities_to_json, RetrievedEntity};

use crate::{
    types::llm_enrichment_result::LLMEnrichmentResult,
    utils::llm_instructions::{get_ingress_analysis_schema, INGRESS_ANALYSIS_SYSTEM_MESSAGE},
};

pub struct IngestionEnricher {
    db_client: Arc<SurrealDbClient>,
    openai_client: Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
}

impl IngestionEnricher {
    pub const fn new(
        db_client: Arc<SurrealDbClient>,
        openai_client: Arc<async_openai::Client<async_openai::config::OpenAIConfig>>,
    ) -> Self {
        Self {
            db_client,
            openai_client,
        }
    }

    pub async fn analyze_content(
        &self,
        category: &str,
        context: Option<&str>,
        text: &str,
        user_id: &str,
    ) -> Result<LLMEnrichmentResult, AppError> {
        let similar_entities = self
            .find_similar_entities(category, context, text, user_id)
            .await?;
        let llm_request = self
            .prepare_llm_request(category, context, text, &similar_entities)
            .await?;
        self.perform_analysis(llm_request).await
    }

    async fn find_similar_entities(
        &self,
        category: &str,
        context: Option<&str>,
        text: &str,
        user_id: &str,
    ) -> Result<Vec<RetrievedEntity>, AppError> {
        let input_text =
            format!("content: {text}, category: {category}, user_context: {context:?}");

        retrieve_entities(&self.db_client, &self.openai_client, &input_text, user_id).await
    }

    async fn prepare_llm_request(
        &self,
        category: &str,
        context: Option<&str>,
        text: &str,
        similar_entities: &[RetrievedEntity],
    ) -> Result<CreateChatCompletionRequest, AppError> {
        let settings = SystemSettings::get_current(&self.db_client).await?;

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
