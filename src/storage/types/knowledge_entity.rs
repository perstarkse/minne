use crate::{
    error::AppError, storage::db::SurrealDbClient, stored_object,
    utils::embedding::generate_embedding,
};
use async_openai::{
    config::{Config, OpenAIConfig},
    Client,
};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum KnowledgeEntityType {
    Idea,
    Project,
    Document,
    Page,
    TextSnippet,
    // Add more types as needed
}

impl From<String> for KnowledgeEntityType {
    fn from(s: String) -> Self {
        match s.to_lowercase().as_str() {
            "idea" => KnowledgeEntityType::Idea,
            "project" => KnowledgeEntityType::Project,
            "document" => KnowledgeEntityType::Document,
            "page" => KnowledgeEntityType::Page,
            "textsnippet" => KnowledgeEntityType::TextSnippet,
            _ => KnowledgeEntityType::Document, // Default case
        }
    }
}

stored_object!(KnowledgeEntity, "knowledge_entity", {
    source_id: String,
    name: String,
    description: String,
    entity_type: KnowledgeEntityType,
    metadata: Option<serde_json::Value>,
    embedding: Vec<f32>,
    user_id: String
});

impl KnowledgeEntity {
    pub fn new(
        source_id: String,
        name: String,
        description: String,
        entity_type: KnowledgeEntityType,
        metadata: Option<serde_json::Value>,
        embedding: Vec<f32>,
        user_id: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            source_id,
            name,
            description,
            entity_type,
            metadata,
            embedding,
            user_id,
        }
    }

    pub async fn delete_by_source_id(
        source_id: &str,
        db_client: &SurrealDbClient,
    ) -> Result<(), AppError> {
        let query = format!(
            "DELETE {} WHERE source_id = '{}'",
            Self::table_name(),
            source_id
        );
        db_client.query(query).await?;

        Ok(())
    }

    pub async fn patch(
        id: &str,
        name: &str,
        description: &str,
        entity_type: &KnowledgeEntityType,
        db_client: &SurrealDbClient,
        ai_client: &Client<OpenAIConfig>,
    ) -> Result<(), AppError> {
        let embedding_input = format!(
            "name: {}, description: {}, type: {:?}",
            name, description, entity_type
        );
        let embedding = generate_embedding(ai_client, &embedding_input).await?;

        db_client
            .client
            .query(
                "UPDATE type::thing($table, $id)
                SET name = $name,
                    description = $description,
                    updated_at = $updated_at,
                    embedding = $embedding
                RETURN AFTER",
            )
            .bind(("table", Self::table_name()))
            .bind(("id", id.to_string()))
            .bind(("name", name.to_string()))
            .bind(("updated_at", Utc::now()))
            .bind(("embedding", embedding))
            .bind(("description", description.to_string()))
            .await?;

        Ok(())
    }
}
