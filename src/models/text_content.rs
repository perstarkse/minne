use crate::retrieval::graph::find_entities_by_source_id;
use crate::retrieval::vector::find_items_by_vector_similarity;
use crate::storage::db::store_item;
use crate::storage::types::knowledge_entity::KnowledgeEntity;
use crate::storage::types::knowledge_relationship::KnowledgeRelationship;
use crate::storage::types::text_chunk::TextChunk;
use crate::storage::types::text_content::TextContent;
use crate::storage::types::StoredObject;
use crate::utils::embedding::generate_embedding;
use crate::{error::ProcessingError, surrealdb::SurrealDbClient, utils::llm::create_json_ld};
use surrealdb::{engine::remote::ws::Client, Surreal};
use text_splitter::TextSplitter;
use tracing::{debug, info};

impl TextContent {
    /// Processes the `TextContent` by sending it to an LLM, storing in a graph DB, and vector DB.
    pub async fn process(&self) -> Result<(), ProcessingError> {
        let db_client = SurrealDbClient::new().await?;
        let openai_client = async_openai::Client::new();

        // Store TextContent
        let create_operation = store_item(&db_client, self.clone()).await?;
        info!("{:?}", create_operation);

        // Get related nodes
        let closest_text_content: Vec<TextChunk> = find_items_by_vector_similarity(
            3,
            self.text.clone(),
            &db_client,
            "text_chunk".to_string(),
            &openai_client,
        )
        .await?;

        for node in closest_text_content {
            let related_nodes: Vec<KnowledgeEntity> = find_entities_by_source_id(
                node.source_id.to_owned(),
                KnowledgeEntity::table_name().to_string(),
                &db_client,
            )
            .await?;
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
        db_client.rebuild_indexes().await?;

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
            debug!(
                "{:?}, {:?}, {:?}",
                &entity.id, &entity.name, &entity.description
            );

            store_item(db_client, entity.clone()).await?;
        }

        for relationship in &relationships {
            debug!("{:?}", relationship);

            store_item(db_client, relationship.clone()).await?;
        }

        info!(
            "Inserted to database: {:?} entities, {:?} relationships",
            entities.len(),
            relationships.len()
        );

        Ok(())
    }

    /// Splits text and stores it in a vector database.
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

            store_item(db_client, text_chunk).await?;
        }

        Ok(())
    }
}
