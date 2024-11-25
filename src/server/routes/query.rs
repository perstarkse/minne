use crate::{
    error::ApiError, retrieval::combined_knowledge_entity_retrieval, storage::db::SurrealDbClient,
};
use async_openai::types::{
    ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
    CreateChatCompletionRequestArgs,
};
use axum::{response::IntoResponse, Extension, Json};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use tracing::info;

#[derive(Debug, Deserialize)]
pub struct QueryInput {
    query: String,
}

pub async fn query_handler(
    Extension(db_client): Extension<Arc<SurrealDbClient>>,
    Json(query): Json<QueryInput>,
) -> Result<impl IntoResponse, ApiError> {
    info!("Received input: {:?}", query);
    let openai_client = async_openai::Client::new();

    let entities =
        combined_knowledge_entity_retrieval(&db_client, &openai_client, query.query.clone())
            .await?;

    let entities_json = json!(entities
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

    let system_message = r#"
      You are a knowledgeable assistant with access to a specialized knowledge base. You will be provided with relevant knowledge entities from the database as context. Each knowledge entity contains a name, description, and type, representing different concepts, ideas, and information.

      Your task is to:
      1. Carefully analyze the provided knowledge entities in the context
      2. Answer user questions based on this information
      3. Provide clear, concise, and accurate responses
      4. When referencing information, briefly mention which knowledge entity it came from
      5. If the provided context doesn't contain enough information to answer the question confidently, clearly state this
      6. If only partial information is available, explain what you can answer and what information is missing
      7. Avoid making assumptions or providing information not supported by the context

      Remember:
      - Be direct and honest about the limitations of your knowledge
      - Cite the relevant knowledge entities when providing information
      - If you need to combine information from multiple entities, explain how they connect
      - Don't speculate beyond what's provided in the context

      Example response formats:
      "Based on [Entity Name], [answer...]"
      "I found relevant information in multiple entries: [explanation...]"
      "I apologize, but the provided context doesn't contain information about [topic]"  
    "#;

    let user_message = format!(
        r#"
        Context Information:
        ==================
        {}

        User Question:
        ==================
        {}
        "#,
        entities_json, query.query
    );

    info!("{:?}", user_message);

    let request = CreateChatCompletionRequestArgs::default()
        .model("gpt-4o-mini")
        .temperature(0.2)
        .max_tokens(3048u32)
        .messages([
            ChatCompletionRequestSystemMessage::from(system_message).into(),
            ChatCompletionRequestUserMessage::from(user_message).into(),
        ])
        .build()?;

    let response = openai_client.chat().create(request).await?;

    let answer = response
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_ref())
        .ok_or(ApiError::QueryError(
            "No content found in LLM response".to_string(),
        ))?;

    info!("{:?}", answer);

    // info!("{:#?}", entities_json);

    Ok(answer.clone().into_response())
}
