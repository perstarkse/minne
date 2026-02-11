#![allow(clippy::arithmetic_side_effects, clippy::missing_docs_in_private_items)]

use std::collections::HashSet;

use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{knowledge_entity::KnowledgeEntity, text_chunk::TextChunk, StoredObject},
    },
};
use retrieval_pipeline::StrategyOutput;
use uuid::Uuid;

pub(crate) const MAX_REFERENCE_COUNT: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InvalidReferenceReason {
    Empty,
    UnsupportedPrefix,
    MalformedUuid,
    Duplicate,
    NotInContext,
    NotFound,
    WrongUser,
    OverLimit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InvalidReference {
    pub raw: String,
    pub normalized: Option<String>,
    pub reason: InvalidReferenceReason,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ReferenceReasonStats {
    pub total: usize,
    pub empty: usize,
    pub unsupported_prefix: usize,
    pub malformed_uuid: usize,
    pub duplicate: usize,
    pub not_in_context: usize,
    pub not_found: usize,
    pub wrong_user: usize,
    pub over_limit: usize,
}

impl ReferenceReasonStats {
    fn record(&mut self, reason: &InvalidReferenceReason) {
        match reason {
            InvalidReferenceReason::Empty => self.empty += 1,
            InvalidReferenceReason::UnsupportedPrefix => self.unsupported_prefix += 1,
            InvalidReferenceReason::MalformedUuid => self.malformed_uuid += 1,
            InvalidReferenceReason::Duplicate => self.duplicate += 1,
            InvalidReferenceReason::NotInContext => self.not_in_context += 1,
            InvalidReferenceReason::NotFound => self.not_found += 1,
            InvalidReferenceReason::WrongUser => self.wrong_user += 1,
            InvalidReferenceReason::OverLimit => self.over_limit += 1,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ReferenceValidationResult {
    pub valid_refs: Vec<String>,
    pub invalid_refs: Vec<InvalidReference>,
    pub reason_stats: ReferenceReasonStats,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReferenceLookupTarget {
    TextChunk,
    KnowledgeEntity,
    Any,
}

pub(crate) fn collect_reference_ids_from_retrieval(
    retrieval_result: &StrategyOutput,
) -> Vec<String> {
    let mut ids = Vec::new();
    let mut seen = HashSet::new();

    match retrieval_result {
        StrategyOutput::Chunks(chunks) => {
            for chunk in chunks {
                let id = chunk.chunk.id.clone();
                if seen.insert(id.clone()) {
                    ids.push(id);
                }
            }
        }
        StrategyOutput::Entities(entities) => {
            for entity in entities {
                let id = entity.entity.id.clone();
                if seen.insert(id.clone()) {
                    ids.push(id);
                }
            }
        }
        StrategyOutput::Search(search) => {
            for chunk in &search.chunks {
                let id = chunk.chunk.id.clone();
                if seen.insert(id.clone()) {
                    ids.push(id);
                }
            }
            for entity in &search.entities {
                let id = entity.entity.id.clone();
                if seen.insert(id.clone()) {
                    ids.push(id);
                }
            }
        }
    }

    ids
}

pub(crate) async fn validate_references(
    user_id: &str,
    refs: Vec<String>,
    allowed_ids: &[String],
    db: &SurrealDbClient,
) -> Result<ReferenceValidationResult, AppError> {
    let mut result = ReferenceValidationResult::default();
    result.reason_stats.total = refs.len();

    let mut seen = HashSet::new();
    let allowed_set: HashSet<&str> = allowed_ids.iter().map(String::as_str).collect();
    let enforce_context = !allowed_set.is_empty();

    for raw in refs {
        let (normalized, target) = match normalize_reference(&raw) {
            Ok(parsed) => parsed,
            Err(reason) => {
                result.reason_stats.record(&reason);
                result.invalid_refs.push(InvalidReference {
                    raw,
                    normalized: None,
                    reason,
                });
                continue;
            }
        };

        if !seen.insert(normalized.clone()) {
            let reason = InvalidReferenceReason::Duplicate;
            result.reason_stats.record(&reason);
            result.invalid_refs.push(InvalidReference {
                raw,
                normalized: Some(normalized),
                reason,
            });
            continue;
        }

        if result.valid_refs.len() >= MAX_REFERENCE_COUNT {
            let reason = InvalidReferenceReason::OverLimit;
            result.reason_stats.record(&reason);
            result.invalid_refs.push(InvalidReference {
                raw,
                normalized: Some(normalized),
                reason,
            });
            continue;
        }

        if enforce_context && !allowed_set.contains(normalized.as_str()) {
            let reason = InvalidReferenceReason::NotInContext;
            result.reason_stats.record(&reason);
            result.invalid_refs.push(InvalidReference {
                raw,
                normalized: Some(normalized),
                reason,
            });
            continue;
        }

        match lookup_reference_for_user(&normalized, &target, user_id, db).await? {
            LookupResult::Found => result.valid_refs.push(normalized),
            LookupResult::WrongUser => {
                let reason = InvalidReferenceReason::WrongUser;
                result.reason_stats.record(&reason);
                result.invalid_refs.push(InvalidReference {
                    raw,
                    normalized: Some(normalized),
                    reason,
                });
            }
            LookupResult::NotFound => {
                let reason = InvalidReferenceReason::NotFound;
                result.reason_stats.record(&reason);
                result.invalid_refs.push(InvalidReference {
                    raw,
                    normalized: Some(normalized),
                    reason,
                });
            }
        }
    }

    Ok(result)
}

pub(crate) fn normalize_reference(
    raw: &str,
) -> Result<(String, ReferenceLookupTarget), InvalidReferenceReason> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(InvalidReferenceReason::Empty);
    }

    let (candidate, target) = if let Some((prefix, rest)) = trimmed.split_once(':') {
        let lookup_target = if prefix.eq_ignore_ascii_case("knowledge_entity") {
            ReferenceLookupTarget::KnowledgeEntity
        } else if prefix.eq_ignore_ascii_case("text_chunk") {
            ReferenceLookupTarget::TextChunk
        } else {
            return Err(InvalidReferenceReason::UnsupportedPrefix);
        };

        (rest.trim(), lookup_target)
    } else {
        (trimmed, ReferenceLookupTarget::Any)
    };

    if candidate.is_empty() {
        return Err(InvalidReferenceReason::MalformedUuid);
    }

    Uuid::parse_str(candidate)
        .map(|uuid| (uuid.to_string(), target))
        .map_err(|_| InvalidReferenceReason::MalformedUuid)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LookupResult {
    Found,
    WrongUser,
    NotFound,
}

async fn lookup_reference_for_user(
    id: &str,
    target: &ReferenceLookupTarget,
    user_id: &str,
    db: &SurrealDbClient,
) -> Result<LookupResult, AppError> {
    match target {
        ReferenceLookupTarget::TextChunk => lookup_single_type::<TextChunk>(id, user_id, db).await,
        ReferenceLookupTarget::KnowledgeEntity => {
            lookup_single_type::<KnowledgeEntity>(id, user_id, db).await
        }
        ReferenceLookupTarget::Any => {
            let chunk_result = lookup_single_type::<TextChunk>(id, user_id, db).await?;
            if chunk_result == LookupResult::Found {
                return Ok(LookupResult::Found);
            }

            let entity_result = lookup_single_type::<KnowledgeEntity>(id, user_id, db).await?;
            if entity_result == LookupResult::Found {
                return Ok(LookupResult::Found);
            }

            if chunk_result == LookupResult::WrongUser || entity_result == LookupResult::WrongUser {
                return Ok(LookupResult::WrongUser);
            }

            Ok(LookupResult::NotFound)
        }
    }
}

async fn lookup_single_type<T>(
    id: &str,
    user_id: &str,
    db: &SurrealDbClient,
) -> Result<LookupResult, AppError>
where
    T: StoredObject + for<'de> serde::Deserialize<'de> + HasUserId,
{
    let item = db.get_item::<T>(id).await?;
    Ok(match item {
        Some(item) if item.user_id() == user_id => LookupResult::Found,
        Some(_) => LookupResult::WrongUser,
        None => LookupResult::NotFound,
    })
}

trait HasUserId {
    fn user_id(&self) -> &str;
}

impl HasUserId for TextChunk {
    fn user_id(&self) -> &str {
        &self.user_id
    }
}

impl HasUserId for KnowledgeEntity {
    fn user_id(&self) -> &str {
        &self.user_id
    }
}

#[cfg(test)]
#[allow(
    clippy::cloned_ref_to_slice_refs,
    clippy::expect_used,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use common::storage::types::knowledge_entity::KnowledgeEntityType;
    use surrealdb::engine::any::connect;

    async fn setup_test_db() -> SurrealDbClient {
        let client = connect("mem://")
            .await
            .expect("failed to create in-memory surrealdb client");
        let namespace = format!("test_ns_{}", Uuid::new_v4());
        let database = format!("test_db_{}", Uuid::new_v4());
        client
            .use_ns(namespace)
            .use_db(database)
            .await
            .expect("failed to select namespace/db");

        let db = SurrealDbClient { client };
        db.apply_migrations()
            .await
            .expect("failed to apply migrations");
        db
    }

    #[tokio::test]
    async fn valid_uuid_exists_and_belongs_to_user() {
        let db = setup_test_db().await;
        let user_id = "user-a";
        let entity = KnowledgeEntity::new(
            "source-1".to_string(),
            "Entity A".to_string(),
            "Entity description".to_string(),
            KnowledgeEntityType::Document,
            None,
            user_id.to_string(),
        );
        db.store_item(entity.clone())
            .await
            .expect("failed to store entity");

        let result =
            validate_references(user_id, vec![entity.id.clone()], &[entity.id.clone()], &db)
                .await
                .expect("validation should not fail");

        assert_eq!(result.valid_refs, vec![entity.id]);
        assert!(result.invalid_refs.is_empty());
    }

    #[tokio::test]
    async fn valid_uuid_exists_but_wrong_user_is_rejected() {
        let db = setup_test_db().await;
        let entity = KnowledgeEntity::new(
            "source-1".to_string(),
            "Entity B".to_string(),
            "Entity description".to_string(),
            KnowledgeEntityType::Document,
            None,
            "other-user".to_string(),
        );
        db.store_item(entity.clone())
            .await
            .expect("failed to store entity");

        let result =
            validate_references("user-a", vec![entity.id.clone()], &[entity.id.clone()], &db)
                .await
                .expect("validation should not fail");

        assert!(result.valid_refs.is_empty());
        assert_eq!(result.invalid_refs.len(), 1);
        assert_eq!(
            result.invalid_refs[0].reason,
            InvalidReferenceReason::WrongUser
        );
    }

    #[tokio::test]
    async fn malformed_uuid_is_rejected() {
        let db = setup_test_db().await;
        let result = validate_references(
            "user-a",
            vec!["not-a-uuid".to_string()],
            &["not-a-uuid".to_string()],
            &db,
        )
        .await
        .expect("validation should not fail");

        assert!(result.valid_refs.is_empty());
        assert_eq!(result.invalid_refs.len(), 1);
        assert_eq!(
            result.invalid_refs[0].reason,
            InvalidReferenceReason::MalformedUuid
        );
    }

    #[tokio::test]
    async fn mixed_duplicates_are_deduped() {
        let db = setup_test_db().await;
        let user_id = "user-a";

        let first = KnowledgeEntity::new(
            "source-1".to_string(),
            "Entity 1".to_string(),
            "Entity description".to_string(),
            KnowledgeEntityType::Document,
            None,
            user_id.to_string(),
        );
        let second = KnowledgeEntity::new(
            "source-2".to_string(),
            "Entity 2".to_string(),
            "Entity description".to_string(),
            KnowledgeEntityType::Document,
            None,
            user_id.to_string(),
        );
        db.store_item(first.clone())
            .await
            .expect("failed to store first entity");
        db.store_item(second.clone())
            .await
            .expect("failed to store second entity");

        let refs = vec![
            first.id.clone(),
            format!("knowledge_entity:{}", first.id),
            second.id.clone(),
            second.id.clone(),
        ];

        let allowed = vec![first.id.clone(), second.id.clone()];
        let result = validate_references(user_id, refs, &allowed, &db)
            .await
            .expect("validation should not fail");

        assert_eq!(result.valid_refs, vec![first.id, second.id]);
        assert_eq!(result.invalid_refs.len(), 2);
        assert!(result
            .invalid_refs
            .iter()
            .all(|entry| entry.reason == InvalidReferenceReason::Duplicate));
    }

    #[tokio::test]
    async fn bare_uuid_prefers_chunk_lookup_before_entity() {
        let db = setup_test_db().await;
        let user_id = "user-a";
        let chunk = TextChunk::new(
            "source-1".to_string(),
            "Chunk body".to_string(),
            user_id.to_string(),
        );
        db.store_item(chunk.clone())
            .await
            .expect("failed to store chunk");

        let result = validate_references(user_id, vec![chunk.id.clone()], &[chunk.id.clone()], &db)
            .await
            .expect("validation should not fail");

        assert_eq!(result.valid_refs, vec![chunk.id]);
    }
}
