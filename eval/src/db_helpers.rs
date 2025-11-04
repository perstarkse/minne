use anyhow::{Context, Result};
use common::storage::db::SurrealDbClient;

// Remove and recreate HNSW indexes for changing embedding lengths, used at beginning if embedding length differs from default system settings
pub async fn change_embedding_length_in_hnsw_indexes(
    db: &SurrealDbClient,
    dimension: usize,
) -> Result<()> {
    tracing::info!("Changing embedding length in HNSW indexes");
    let query = format!(
        "BEGIN TRANSACTION;
        REMOVE INDEX IF EXISTS idx_embedding_chunks ON TABLE text_chunk;
        REMOVE INDEX IF EXISTS idx_embedding_entities ON TABLE knowledge_entity;
        DEFINE INDEX idx_embedding_chunks ON TABLE text_chunk FIELDS embedding HNSW DIMENSION {dim};
        DEFINE INDEX idx_embedding_entities ON TABLE knowledge_entity FIELDS embedding HNSW DIMENSION {dim};
        COMMIT TRANSACTION;",
        dim = dimension
    );

    db.client
        .query(query)
        .await
        .context("changing HNSW indexes")?;
    tracing::info!("HNSW indexes successfully changed");
    Ok(())
}

// Helper functions for index management during namespace reseed
pub async fn remove_all_indexes(db: &SurrealDbClient) -> Result<()> {
    tracing::info!("Removing ALL indexes before namespace reseed (aggressive approach)");

    // Remove ALL indexes from ALL tables to ensure no cache access
    db.client
        .query(
            "BEGIN TRANSACTION;
            -- HNSW indexes
            REMOVE INDEX IF EXISTS idx_embedding_chunks ON TABLE text_chunk;
            REMOVE INDEX IF EXISTS idx_embedding_entities ON TABLE knowledge_entity;

            -- FTS indexes on text_content (remove ALL of them)
            REMOVE INDEX IF EXISTS text_content_fts_idx ON TABLE text_content;
            REMOVE INDEX IF EXISTS text_content_fts_text_idx ON TABLE text_content;
            REMOVE INDEX IF EXISTS text_content_fts_category_idx ON TABLE text_content;
            REMOVE INDEX IF EXISTS text_content_fts_context_idx ON TABLE text_content;
            REMOVE INDEX IF EXISTS text_content_fts_file_name_idx ON TABLE text_content;
            REMOVE INDEX IF EXISTS text_content_fts_url_idx ON TABLE text_content;
            REMOVE INDEX IF EXISTS text_content_fts_url_title_idx ON TABLE text_content;

            -- FTS indexes on knowledge_entity
            REMOVE INDEX IF EXISTS knowledge_entity_fts_name_idx ON TABLE knowledge_entity;
            REMOVE INDEX IF EXISTS knowledge_entity_fts_description_idx ON TABLE knowledge_entity;

            -- FTS indexes on text_chunk
            REMOVE INDEX IF EXISTS text_chunk_fts_chunk_idx ON TABLE text_chunk;

            COMMIT TRANSACTION;",
        )
        .await
        .context("removing all indexes before namespace reseed")?;

    tracing::info!("All indexes removed before namespace reseed");
    Ok(())
}

async fn create_tokenizer(db: &SurrealDbClient) -> Result<()> {
    tracing::info!("Creating FTS analyzers for namespace reseed");
    let res = db
        .client
        .query(
            "BEGIN TRANSACTION;
            DEFINE ANALYZER IF NOT EXISTS app_en_fts_analyzer
                TOKENIZERS class
                FILTERS lowercase, ascii, snowball(english);
            COMMIT TRANSACTION;",
        )
        .await
        .context("creating FTS analyzers for namespace reseed")?;

    res.check().context("failed to create the tokenizer")?;
    Ok(())
}

pub async fn recreate_indexes(db: &SurrealDbClient, dimension: usize) -> Result<()> {
    tracing::info!("Recreating ALL indexes after namespace reseed (SEQUENTIAL approach)");
    let total_start = std::time::Instant::now();

    create_tokenizer(db)
        .await
        .context("creating FTS analyzer")?;

    // For now we dont remove these plain indexes, we could if they prove negatively impacting performance
    // create_regular_indexes_for_snapshot(db)
    //     .await
    //     .context("creating regular indexes for namespace reseed")?;

    let fts_start = std::time::Instant::now();
    create_fts_indexes_for_snapshot(db)
        .await
        .context("creating FTS indexes for namespace reseed")?;
    tracing::info!(duration = ?fts_start.elapsed(), "FTS indexes created");

    let hnsw_start = std::time::Instant::now();
    create_hnsw_indexes_for_snapshot(db, dimension)
        .await
        .context("creating HNSW indexes for namespace reseed")?;
    tracing::info!(duration = ?hnsw_start.elapsed(), "HNSW indexes created");

    tracing::info!(duration = ?total_start.elapsed(), "All index groups recreated successfully in sequence");
    Ok(())
}

#[allow(dead_code)] // For now we dont do this. We could
async fn create_regular_indexes_for_snapshot(db: &SurrealDbClient) -> Result<()> {
    tracing::info!("Creating regular indexes for namespace reseed (parallel group 1)");
    let res = db
        .client
        .query(
            "BEGIN TRANSACTION;
            DEFINE INDEX text_content_user_id_idx ON text_content FIELDS user_id;
            DEFINE INDEX text_content_created_at_idx ON text_content FIELDS created_at;
            DEFINE INDEX text_content_category_idx ON text_content FIELDS category;
            DEFINE INDEX text_chunk_source_id_idx ON text_chunk FIELDS source_id;
            DEFINE INDEX text_chunk_user_id_idx ON text_chunk FIELDS user_id;
            DEFINE INDEX knowledge_entity_user_id_idx ON knowledge_entity FIELDS user_id;
            DEFINE INDEX knowledge_entity_source_id_idx ON knowledge_entity FIELDS source_id;
            DEFINE INDEX knowledge_entity_entity_type_idx ON knowledge_entity FIELDS entity_type;
            DEFINE INDEX knowledge_entity_created_at_idx ON knowledge_entity FIELDS created_at;
            COMMIT TRANSACTION;",
        )
        .await
        .context("creating regular indexes for namespace reseed")?;

    res.check().context("one of the regular indexes failed")?;

    tracing::info!("Regular indexes for namespace reseed created");
    Ok(())
}

async fn create_fts_indexes_for_snapshot(db: &SurrealDbClient) -> Result<()> {
    tracing::info!("Creating FTS indexes for namespace reseed (group 2)");
    let res = db.client
        .query(
            "BEGIN TRANSACTION;
             DEFINE INDEX text_content_fts_idx ON TABLE text_content FIELDS text;
             DEFINE INDEX knowledge_entity_fts_name_idx ON TABLE knowledge_entity FIELDS name
                 SEARCH ANALYZER app_en_fts_analyzer BM25;
             DEFINE INDEX knowledge_entity_fts_description_idx ON TABLE knowledge_entity FIELDS description
                 SEARCH ANALYZER app_en_fts_analyzer BM25;
             DEFINE INDEX text_chunk_fts_chunk_idx ON TABLE text_chunk FIELDS chunk
                 SEARCH ANALYZER app_en_fts_analyzer BM25;
             COMMIT TRANSACTION;",
        )
        .await
        .context("sending FTS index creation query")?;

    // This actually surfaces statement-level errors
    res.check()
        .context("one or more FTS index statements failed")?;

    tracing::info!("FTS indexes for namespace reseed created");
    Ok(())
}

async fn create_hnsw_indexes_for_snapshot(db: &SurrealDbClient, dimension: usize) -> Result<()> {
    tracing::info!("Creating HNSW indexes for namespace reseed (group 3)");
    let query = format!(
        "BEGIN TRANSACTION;
        DEFINE INDEX idx_embedding_chunks ON TABLE text_chunk FIELDS embedding HNSW DIMENSION {dim};
        DEFINE INDEX idx_embedding_entities ON TABLE knowledge_entity FIELDS embedding HNSW DIMENSION {dim};
        COMMIT TRANSACTION;",
        dim = dimension
    );

    let res = db
        .client
        .query(query)
        .await
        .context("creating HNSW indexes for namespace reseed")?;

    res.check()
        .context("one or more HNSW index statements failed")?;

    tracing::info!("HNSW indexes for namespace reseed created");
    Ok(())
}

pub async fn reset_namespace(db: &SurrealDbClient, namespace: &str, database: &str) -> Result<()> {
    let query = format!(
        "REMOVE NAMESPACE {ns};
         DEFINE NAMESPACE {ns};
         DEFINE DATABASE {db};",
        ns = namespace,
        db = database
    );
    db.client
        .query(query)
        .await
        .context("resetting SurrealDB namespace")?;
    db.client
        .use_ns(namespace)
        .use_db(database)
        .await
        .context("selecting namespace/database after reset")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use uuid::Uuid;

    #[derive(Debug, Deserialize)]
    struct FooRow {
        label: String,
    }

    #[tokio::test]
    async fn reset_namespace_drops_existing_rows() {
        let namespace = format!("reset_ns_{}", Uuid::new_v4().simple());
        let database = format!("reset_db_{}", Uuid::new_v4().simple());
        let db = SurrealDbClient::memory(&namespace, &database)
            .await
            .expect("in-memory db");

        db.client
            .query(
                "DEFINE TABLE foo SCHEMALESS;
                 CREATE foo:foo SET label = 'before';",
            )
            .await
            .expect("seed namespace")
            .check()
            .expect("seed response");

        let mut before = db
            .client
            .query("SELECT * FROM foo")
            .await
            .expect("select before reset");
        let existing: Vec<FooRow> = before.take(0).expect("rows before reset");
        assert_eq!(existing.len(), 1);
        assert_eq!(existing[0].label, "before");

        reset_namespace(&db, &namespace, &database)
            .await
            .expect("namespace reset");

        match db.client.query("SELECT * FROM foo").await {
            Ok(mut response) => {
                let rows: Vec<FooRow> = response.take(0).unwrap_or_default();
                assert!(
                    rows.is_empty(),
                    "reset namespace should drop rows, found {:?}",
                    rows
                );
            }
            Err(error) => {
                let message = error.to_string();
                assert!(
                    message.to_ascii_lowercase().contains("table")
                        || message.to_ascii_lowercase().contains("namespace")
                        || message.to_ascii_lowercase().contains("foo"),
                    "unexpected error after namespace reset: {message}"
                );
            }
        }
    }
}
