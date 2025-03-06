use crate::{
    error::AppError,
    ingress::analysis::prompt::{get_ingress_analysis_schema, INGRESS_ANALYSIS_SYSTEM_MESSAGE},
    retrieval::combined_knowledge_entity_retrieval,
    storage::{db::SurrealDbClient, types::knowledge_entity::KnowledgeEntity},
};
use async_openai::{
    error::OpenAIError,
    types::{
        ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
        CreateChatCompletionRequest, CreateChatCompletionRequestArgs, ResponseFormat,
        ResponseFormatJsonSchema,
    },
};
use serde_json::json;
use tracing::debug;

use super::types::llm_analysis_result::LLMGraphAnalysisResult;

pub struct IngressAnalyzer<'a> {
    db_client: &'a SurrealDbClient,
    openai_client: &'a async_openai::Client<async_openai::config::OpenAIConfig>,
}

impl<'a> IngressAnalyzer<'a> {
    pub fn new(
        db_client: &'a SurrealDbClient,
        openai_client: &'a async_openai::Client<async_openai::config::OpenAIConfig>,
    ) -> Self {
        Self {
            db_client,
            openai_client,
        }
    }

    pub async fn analyze_content(
        &self,
        category: &str,
        instructions: &str,
        text: &str,
        user_id: &str,
    ) -> Result<LLMGraphAnalysisResult, AppError> {
        let similar_entities = self
            .find_similar_entities(category, instructions, text, user_id)
            .await?;
        let llm_request =
            self.prepare_llm_request(category, instructions, text, &similar_entities)?;
        self.perform_analysis(llm_request).await
    }

    async fn find_similar_entities(
        &self,
        category: &str,
        instructions: &str,
        text: &str,
        user_id: &str,
    ) -> Result<Vec<KnowledgeEntity>, AppError> {
        let input_text = format!(
            "content: {}, category: {}, user_instructions: {}",
            text, category, instructions
        );

        combined_knowledge_entity_retrieval(
            self.db_client,
            self.openai_client,
            &input_text,
            user_id,
        )
        .await
    }

    fn prepare_llm_request(
        &self,
        category: &str,
        instructions: &str,
        text: &str,
        similar_entities: &[KnowledgeEntity],
    ) -> Result<CreateChatCompletionRequest, OpenAIError> {
        let entities_json = json!(similar_entities
            .iter()
            .map(|entity| {
                json!({
                    "KnowledgeEntity": {
                        "id": entity.id,
                        "name": entity.name,
                        "description": entity.description
                    }
                })
            })
            .collect::<Vec<_>>());

        let user_message = format!(
            "Category:\n{}\nInstructions:\n{}\nContent:\n{}\nExisting KnowledgeEntities in database:\n{}",
            category, instructions, text, entities_json
        );

        debug!("Prepared LLM request message: {}", user_message);

        let response_format = ResponseFormat::JsonSchema {
            json_schema: ResponseFormatJsonSchema {
                description: Some("Structured analysis of the submitted content".into()),
                name: "content_analysis".into(),
                schema: Some(get_ingress_analysis_schema()),
                strict: Some(true),
            },
        };

        CreateChatCompletionRequestArgs::default()
            .model("gpt-4o-mini")
            .temperature(0.2)
            .max_tokens(3048u32)
            .messages([
                ChatCompletionRequestSystemMessage::from(INGRESS_ANALYSIS_SYSTEM_MESSAGE).into(),
                ChatCompletionRequestUserMessage::from(user_message).into(),
            ])
            .response_format(response_format)
            .build()
    }

    async fn perform_analysis(
        &self,
        request: CreateChatCompletionRequest,
    ) -> Result<LLMGraphAnalysisResult, AppError> {
        let response = self.openai_client.chat().create(request).await?;
        debug!("Received LLM response: {:?}", response);

        response
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_ref())
            .ok_or(AppError::LLMParsing(
                "No content found in LLM response".to_string(),
            ))
            .and_then(|content| {
                serde_json::from_str(content).map_err(|e| {
                    AppError::LLMParsing(format!(
                        "Failed to parse LLM response into analysis: {}",
                        e
                    ))
                })
            })
    }
}
