use std::collections::HashMap;

use serde::Deserialize;
use tracing::debug;

use common::{
    error::AppError,
    storage::{db::SurrealDbClient, types::StoredObject},
};

use crate::scoring::Scored;
use common::storage::types::file_info::deserialize_flexible_id;
use surrealdb::sql::Thing;

#[derive(Debug, Deserialize)]
struct FtsScoreRow {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    id: String,
    fts_score: Option<f32>,
}

/// Executes a full-text search query against SurrealDB and returns scored results.
///
/// The function expects FTS indexes to exist for the provided table. Currently supports
/// `knowledge_entity` (name + description) and `text_chunk` (chunk).
pub async fn find_items_by_fts<T>(
    take: usize,
    query: &str,
    db_client: &SurrealDbClient,
    table: &str,
    user_id: &str,
) -> Result<Vec<Scored<T>>, AppError>
where
    T: for<'de> serde::Deserialize<'de> + StoredObject,
{
    let (filter_clause, score_clause) = match table {
        "knowledge_entity" => (
            "(name @0@ $terms OR description @1@ $terms)",
            "(IF search::score(0) != NONE THEN search::score(0) ELSE 0 END) + \
             (IF search::score(1) != NONE THEN search::score(1) ELSE 0 END)",
        ),
        "text_chunk" => (
            "(chunk @0@ $terms)",
            "IF search::score(0) != NONE THEN search::score(0) ELSE 0 END",
        ),
        _ => {
            return Err(AppError::Validation(format!(
                "FTS not configured for table '{table}'"
            )))
        }
    };

    let sql = format!(
        "SELECT id, {score_clause} AS fts_score \
         FROM {table} \
         WHERE {filter_clause} \
           AND user_id = $user_id \
         ORDER BY fts_score DESC \
         LIMIT $limit",
        table = table,
        filter_clause = filter_clause,
        score_clause = score_clause
    );

    debug!(
        table = table,
        limit = take,
        "Executing FTS query with filter clause: {}",
        filter_clause
    );

    let mut response = db_client
        .query(sql)
        .bind(("terms", query.to_owned()))
        .bind(("user_id", user_id.to_owned()))
        .bind(("limit", take as i64))
        .await?;

    let score_rows: Vec<FtsScoreRow> = response.take(0)?;

    if score_rows.is_empty() {
        return Ok(Vec::new());
    }

    let ids: Vec<String> = score_rows.iter().map(|row| row.id.clone()).collect();
    let thing_ids: Vec<Thing> = ids
        .iter()
        .map(|id| Thing::from((table, id.as_str())))
        .collect();

    let mut items_response = db_client
        .query("SELECT * FROM type::table($table) WHERE id IN $things AND user_id = $user_id")
        .bind(("table", table.to_owned()))
        .bind(("things", thing_ids.clone()))
        .bind(("user_id", user_id.to_owned()))
        .await?;

    let items: Vec<T> = items_response.take(0)?;

    let mut item_map: HashMap<String, T> = items
        .into_iter()
        .map(|item| (item.get_id().to_owned(), item))
        .collect();

    let mut results = Vec::with_capacity(score_rows.len());
    for row in score_rows {
        if let Some(item) = item_map.remove(&row.id) {
            let score = row.fts_score.unwrap_or_default();
            results.push(Scored::new(item).with_fts_score(score));
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::storage::indexes::ensure_runtime_indexes;
    use common::storage::types::{
        knowledge_entity::{KnowledgeEntity, KnowledgeEntityType},
        text_chunk::TextChunk,
        StoredObject,
    };
    use uuid::Uuid;

    #[tokio::test]
    async fn fts_preserves_single_field_score_for_name() {
        let namespace = "fts_test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("failed to create in-memory surreal");

        db.apply_migrations()
            .await
            .expect("failed to apply migrations");
        ensure_runtime_indexes(&db, 1536)
            .await
            .expect("failed to build runtime indexes");

        let user_id = "user_fts";
        let entity = KnowledgeEntity::new(
            "source_a".into(),
            "Rustacean handbook".into(),
            "completely unrelated description".into(),
            KnowledgeEntityType::Document,
            None,
            user_id.into(),
        );

        db.store_item(entity.clone())
            .await
            .expect("failed to insert entity");

        db.rebuild_indexes()
            .await
            .expect("failed to rebuild indexes");

        let results = find_items_by_fts::<KnowledgeEntity>(
            5,
            "rustacean",
            &db,
            KnowledgeEntity::table_name(),
            user_id,
        )
        .await
        .expect("fts query failed");

        assert!(!results.is_empty(), "expected at least one FTS result");
        assert!(
            results[0].scores.fts.is_some(),
            "expected an FTS score when only the name matched"
        );
    }

    #[tokio::test]
    async fn fts_preserves_single_field_score_for_description() {
        let namespace = "fts_test_ns_desc";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("failed to create in-memory surreal");

        db.apply_migrations()
            .await
            .expect("failed to apply migrations");
        ensure_runtime_indexes(&db, 1536)
            .await
            .expect("failed to build runtime indexes");

        let user_id = "user_fts_desc";
        let entity = KnowledgeEntity::new(
            "source_b".into(),
            "neutral name".into(),
            "Detailed notes about async runtimes".into(),
            KnowledgeEntityType::Document,
            None,
            user_id.into(),
        );

        db.store_item(entity.clone())
            .await
            .expect("failed to insert entity");

        db.rebuild_indexes()
            .await
            .expect("failed to rebuild indexes");

        let results = find_items_by_fts::<KnowledgeEntity>(
            5,
            "async",
            &db,
            KnowledgeEntity::table_name(),
            user_id,
        )
        .await
        .expect("fts query failed");

        assert!(!results.is_empty(), "expected at least one FTS result");
        assert!(
            results[0].scores.fts.is_some(),
            "expected an FTS score when only the description matched"
        );
    }

    #[tokio::test]
    async fn fts_preserves_scores_for_text_chunks() {
        let namespace = "fts_test_ns_chunks";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("failed to create in-memory surreal");

        db.apply_migrations()
            .await
            .expect("failed to apply migrations");
        ensure_runtime_indexes(&db, 1536)
            .await
            .expect("failed to build runtime indexes");

        let user_id = "user_fts_chunk";
        let chunk = TextChunk::new(
            "source_chunk".into(),
            "GraphQL documentation reference".into(),
            user_id.into(),
        );

        TextChunk::store_with_embedding(chunk.clone(), vec![0.0; 1536], &db)
            .await
            .expect("failed to insert chunk");

        db.rebuild_indexes()
            .await
            .expect("failed to rebuild indexes");

        let results =
            find_items_by_fts::<TextChunk>(5, "graphql", &db, TextChunk::table_name(), user_id)
                .await
                .expect("fts query failed");

        assert!(!results.is_empty(), "expected at least one FTS result");
        assert!(
            results[0].scores.fts.is_some(),
            "expected an FTS score when chunk field matched"
        );
    }
}
