use crate::storage;
use crate::storage::db::store_item;
use crate::storage::types::text_chunk::TextChunk;
use crate::storage::types::text_content::TextContent;
use crate::{
    error::ProcessingError,
    surrealdb::SurrealDbClient,
    utils::llm::{create_json_ld, generate_embedding},
};
use surrealdb::{engine::remote::ws::Client, Surreal};
use text_splitter::TextSplitter;
use tracing::{debug, info};
use uuid::Uuid;

use super::graph_entities::{KnowledgeEntity, KnowledgeRelationship};

// #[derive(Serialize, Deserialize, Debug)]
// struct TextChunk {
//     #[serde(deserialize_with = "thing_to_string")]
//     id: String,
//     source_id: String,
//     chunk: String,
//     embedding: Vec<f32>,
// }

/// Represents a single piece of text content extracted from various sources.
// #[derive(Debug, Serialize, Deserialize, Clone)]
// pub struct TextContent {
//     #[serde(deserialize_with = "thing_to_string")]
//     pub id: String,
//     pub text: String,
//     pub file_info: Option<FileInfo>,
//     pub instructions: String,
//     pub category: String,
// }

async fn vector_comparison<T>(
    take: u8,
    input_text: String,
    db_client: &Surreal<Client>,
    table: String,
    openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
) -> Result<Vec<T>, ProcessingError>
where
    T: for<'de> serde::Deserialize<'de>,
{
    let input_embedding = generate_embedding(openai_client, input_text).await?;

    // Construct the query
    let closest_query = format!("SELECT *, vector::distance::knn() AS distance FROM {} WHERE embedding <|{},40|> {:?} ORDER BY distance",table, take, input_embedding);

    // Perform query and deserialize to struct
    let closest_entities: Vec<T> = db_client.query(closest_query).await?.take(0)?;

    Ok(closest_entities)
}

async fn get_related_nodes(
    id: String,
    db_client: &Surreal<Client>,
) -> Result<Vec<KnowledgeEntity>, ProcessingError> {
    let query = format!("SELECT * FROM knowledge_entity WHERE source_id = '{}'", id);

    // let query = format!("SELECT * FROM knowledge_entity WHERE in OR out {}", id);
    let related_nodes: Vec<KnowledgeEntity> = db_client.query(query).await?.take(0)?;

    Ok(related_nodes)
}

impl TextContent {
    /// Processes the `TextContent` by sending it to an LLM, storing in a graph DB, and vector DB.
    pub async fn process(&self) -> Result<(), ProcessingError> {
        // Store TextContent
        let db_client = SurrealDbClient::new().await?;
        let openai_client = async_openai::Client::new();

        let create_operation = storage::db::store_item(&db_client, self.clone()).await?;
        info!("{:?}", create_operation);
        // self.store_text_content(&db_client).await?;

        let closest_text_content: Vec<TextChunk> = vector_comparison(
            3,
            self.text.clone(),
            &db_client,
            "text_chunk".to_string(),
            &openai_client,
        )
        .await?;

        for node in closest_text_content {
            let related_nodes = get_related_nodes(node.source_id, &db_client).await?;
            for related_node in related_nodes {
                info!("{:?}", related_node.name);
            }
        }

        // panic!("STOPPING");
        // let deleted: Vec<TextChunk> = db_client.delete("text_chunk").await?;
        // info! {"{:?} KnowledgeEntities deleted", deleted.len()};

        // let relationships_deleted: Vec<KnowledgeRelationship> =
        //     db_client.delete("knowledge_relationship").await?;
        // info!("{:?} Relationships deleted", relationships_deleted.len());

        // panic!("STOP");

        // db_client.query("REMOVE INDEX embeddings ON knowledge_entity").await?;
        // db_client
        //     .query("DEFINE INDEX idx_embedding ON text_chunk FIELDS embedding HNSW DIMENSION 1536")
        //     .await?;
        db_client
            .query("REBUILD INDEX IF EXISTS idx_embedding ON text_chunk")
            .await?;
        db_client
            .query("REBUILD INDEX IF EXISTS embeddings ON knowledge_entity")
            .await?;

        // Step 1: Send to LLM for analysis
        let analysis = create_json_ld(
            &self.category,
            &self.instructions,
            &self.text,
            &db_client,
            &openai_client,
        )
        .await?;
        // info!("{:#?}", &analysis);

        // Step 2: Convert LLM analysis to database entities
        let (entities, relationships) = analysis
            .to_database_entities(&self.id, &openai_client)
            .await?;

        // Step 3: Store in database
        self.store_in_graph_db(entities, relationships, &db_client)
            .await?;

        // Step 4: Split text and store in Vector DB
        self.store_in_vector_db(&db_client, &openai_client).await?;

        Ok(())
    }

    async fn store_in_graph_db(
        &self,
        entities: Vec<KnowledgeEntity>,
        relationships: Vec<KnowledgeRelationship>,
        db_client: &Surreal<Client>,
    ) -> Result<(), ProcessingError> {
        for entity in &entities {
            info!(
                "{:?}, {:?}, {:?}",
                &entity.id, &entity.name, &entity.description
            );

            let _created: Option<KnowledgeEntity> = db_client
                .create(("knowledge_entity", &entity.id.to_string()))
                .content(entity.clone())
                .await?;

            debug!("{:?}", _created);
        }

        for relationship in &relationships {
            // info!("{:?}", relationship);

            let _created: Option<KnowledgeRelationship> = db_client
                .insert(("knowledge_relationship", &relationship.id.to_string()))
                .content(relationship.clone())
                .await?;

            debug!("{:?}", _created);
        }

        // for relationship in &relationships {
        //     let in_entity: Option<KnowledgeEntity> = db_client.select(("knowledge_entity",relationship.in_.to_string())).await?;
        //     let out_entity: Option<KnowledgeEntity> = db_client.select(("knowledge_entity", relationship.out.to_string())).await?;

        //     if let (Some(in_), Some(out)) = (in_entity, out_entity) {
        //     info!("{} - {} is {} to {} - {}", in_.id, in_.name, relationship.relationship_type, out.id, out.name);
        //     }
        //     else {
        //         info!("No in or out entities found");
        //     }
        // }

        info!(
            "Inserted to database: {:?} entities, {:?} relationships",
            entities.len(),
            relationships.len()
        );

        Ok(())
    }

    /// Splits text and stores it in a vector database.
    #[allow(dead_code)]
    async fn store_in_vector_db(
        &self,
        db_client: &Surreal<Client>,
        openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    ) -> Result<(), ProcessingError> {
        let max_characters = 500..2000;
        let splitter = TextSplitter::new(max_characters);

        let chunks = splitter.chunks(self.text.as_str());

        for chunk in chunks {
            info!("Chunk: {}", chunk);
            let embedding = generate_embedding(&openai_client, chunk.to_string()).await?;
            let text_chunk = TextChunk::new(self.id.to_string(), chunk.to_string(), embedding);

            info!("{:?}", text_chunk);

            store_item(db_client, text_chunk).await?;
        }

        Ok(())
    }
}
