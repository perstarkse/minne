use anyhow::{Context, Result};
use common::storage::{db::SurrealDbClient, indexes::ensure_runtime};
use tracing::info;

pub async fn recreate_indexes(db: &SurrealDbClient, dimension: usize) -> Result<()> {
    info!("Recreating ALL indexes after namespace reseed via shared runtime helper");
    ensure_runtime(db, dimension)
        .await
        .context("creating runtime indexes")
}

pub async fn reset_namespace(db: &SurrealDbClient, namespace: &str, database: &str) -> Result<()> {
    let query = format!(
        "REMOVE NAMESPACE {namespace};
         DEFINE NAMESPACE {namespace};
         DEFINE DATABASE {database};"
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

#[allow(clippy::cast_precision_loss)]
pub(crate) async fn warm_hnsw_cache(db: &SurrealDbClient, dimension: usize) -> Result<()> {
    let dummy_embedding: Vec<f32> = (0..dimension).map(|i| (i as f32).sin()).collect();

    info!("Warming HNSW caches with sample queries");

    let _ = db
        .client
        .query(
            r#"SELECT chunk_id
               FROM text_chunk_embedding
               WHERE embedding <|1,1|> $embedding
               LIMIT 5"#,
        )
        .bind(("embedding", dummy_embedding.clone()))
        .await
        .context("warming text chunk HNSW cache")?;

    let _ = db
        .client
        .query(
            r#"SELECT entity_id
               FROM knowledge_entity_embedding
               WHERE embedding <|1,1|> $embedding
               LIMIT 5"#,
        )
        .bind(("embedding", dummy_embedding))
        .await
        .context("warming knowledge entity HNSW cache")?;

    info!("HNSW cache warming completed");
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
    #[allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]
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
                    "reset namespace should drop rows, found {rows:?}",
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
