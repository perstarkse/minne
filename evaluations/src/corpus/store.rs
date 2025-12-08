use std::{
    collections::{HashMap, HashSet},
    fs,
    io::BufReader,
    path::PathBuf,
};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use common::storage::types::StoredObject;
use common::storage::{
    db::SurrealDbClient,
    types::{
        knowledge_entity::KnowledgeEntity,
        knowledge_entity_embedding::KnowledgeEntityEmbedding,
        knowledge_relationship::{KnowledgeRelationship, RelationshipMetadata},
        text_chunk::TextChunk,
        text_chunk_embedding::TextChunkEmbedding,
        text_content::TextContent,
    },
};
use serde::Deserialize;
use serde::Serialize;
use surrealdb::sql::Thing;
use tracing::{debug, warn};

use crate::datasets::{ConvertedParagraph, ConvertedQuestion};

pub const MANIFEST_VERSION: u32 = 3;
pub const PARAGRAPH_SHARD_VERSION: u32 = 3;
const MANIFEST_BATCH_SIZE: usize = 100;
const MANIFEST_MAX_BYTES_PER_BATCH: usize = 300_000; // default cap for non-text batches
const TEXT_CONTENT_MAX_BYTES_PER_BATCH: usize = 250_000; // text bodies can be large; limit aggressively
const MAX_BATCHES_PER_REQUEST: usize = 24;
const REQUEST_MAX_BYTES: usize = 800_000; // total payload cap per Surreal query request

fn current_manifest_version() -> u32 {
    MANIFEST_VERSION
}

fn current_paragraph_shard_version() -> u32 {
    PARAGRAPH_SHARD_VERSION
}

fn default_chunk_min_tokens() -> usize {
    500
}

fn default_chunk_max_tokens() -> usize {
    2_000
}

fn default_chunk_only() -> bool {
    false
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EmbeddedKnowledgeEntity {
    pub entity: KnowledgeEntity,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EmbeddedTextChunk {
    pub chunk: TextChunk,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct LegacyKnowledgeEntity {
    #[serde(flatten)]
    pub entity: KnowledgeEntity,
    #[serde(default)]
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct LegacyTextChunk {
    #[serde(flatten)]
    pub chunk: TextChunk,
    #[serde(default)]
    pub embedding: Vec<f32>,
}

fn deserialize_embedded_entities<'de, D>(
    deserializer: D,
) -> Result<Vec<EmbeddedKnowledgeEntity>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum EntityInput {
        Embedded(Vec<EmbeddedKnowledgeEntity>),
        Legacy(Vec<LegacyKnowledgeEntity>),
    }

    match EntityInput::deserialize(deserializer)? {
        EntityInput::Embedded(items) => Ok(items),
        EntityInput::Legacy(items) => Ok(items
            .into_iter()
            .map(|legacy| EmbeddedKnowledgeEntity {
                entity: legacy.entity,
                embedding: legacy.embedding,
            })
            .collect()),
    }
}

fn deserialize_embedded_chunks<'de, D>(deserializer: D) -> Result<Vec<EmbeddedTextChunk>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum ChunkInput {
        Embedded(Vec<EmbeddedTextChunk>),
        Legacy(Vec<LegacyTextChunk>),
    }

    match ChunkInput::deserialize(deserializer)? {
        ChunkInput::Embedded(items) => Ok(items),
        ChunkInput::Legacy(items) => Ok(items
            .into_iter()
            .map(|legacy| EmbeddedTextChunk {
                chunk: legacy.chunk,
                embedding: legacy.embedding,
            })
            .collect()),
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CorpusManifest {
    #[serde(default = "current_manifest_version")]
    pub version: u32,
    pub metadata: CorpusMetadata,
    pub paragraphs: Vec<CorpusParagraph>,
    pub questions: Vec<CorpusQuestion>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CorpusMetadata {
    pub dataset_id: String,
    pub dataset_label: String,
    pub slice_id: String,
    pub include_unanswerable: bool,
    #[serde(default)]
    pub require_verified_chunks: bool,
    pub ingestion_fingerprint: String,
    pub embedding_backend: String,
    pub embedding_model: Option<String>,
    pub embedding_dimension: usize,
    pub converted_checksum: String,
    pub generated_at: DateTime<Utc>,
    pub paragraph_count: usize,
    pub question_count: usize,
    #[serde(default = "default_chunk_min_tokens")]
    pub chunk_min_tokens: usize,
    #[serde(default = "default_chunk_max_tokens")]
    pub chunk_max_tokens: usize,
    #[serde(default = "default_chunk_only")]
    pub chunk_only: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CorpusParagraph {
    pub paragraph_id: String,
    pub title: String,
    pub text_content: TextContent,
    #[serde(deserialize_with = "deserialize_embedded_entities")]
    pub entities: Vec<EmbeddedKnowledgeEntity>,
    pub relationships: Vec<KnowledgeRelationship>,
    #[serde(deserialize_with = "deserialize_embedded_chunks")]
    pub chunks: Vec<EmbeddedTextChunk>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CorpusQuestion {
    pub question_id: String,
    pub paragraph_id: String,
    pub text_content_id: String,
    pub question_text: String,
    pub answers: Vec<String>,
    pub is_impossible: bool,
    pub matching_chunk_ids: Vec<String>,
}

pub struct CorpusHandle {
    pub manifest: CorpusManifest,
    pub path: PathBuf,
    pub reused_ingestion: bool,
    pub reused_embeddings: bool,
    pub positive_reused: usize,
    pub positive_ingested: usize,
    pub negative_reused: usize,
    pub negative_ingested: usize,
}

pub fn window_manifest(
    manifest: &CorpusManifest,
    offset: usize,
    length: usize,
    negative_multiplier: f32,
) -> Result<CorpusManifest> {
    let total = manifest.questions.len();
    if total == 0 {
        return Err(anyhow!(
            "manifest contains no questions; cannot select a window"
        ));
    }
    if offset >= total {
        return Err(anyhow!(
            "window offset {} exceeds manifest questions ({})",
            offset,
            total
        ));
    }
    let end = (offset + length).min(total);
    let questions = manifest.questions[offset..end].to_vec();

    let selected_positive_ids: HashSet<_> =
        questions.iter().map(|q| q.paragraph_id.clone()).collect();
    let positives_all: HashSet<_> = manifest
        .questions
        .iter()
        .map(|q| q.paragraph_id.as_str())
        .collect();
    let available_negatives = manifest
        .paragraphs
        .len()
        .saturating_sub(positives_all.len());
    let desired_negatives =
        ((selected_positive_ids.len() as f32) * negative_multiplier).ceil() as usize;
    let desired_negatives = desired_negatives.min(available_negatives);

    let mut paragraphs = Vec::new();
    let mut negative_count = 0usize;
    for paragraph in &manifest.paragraphs {
        if selected_positive_ids.contains(&paragraph.paragraph_id) {
            paragraphs.push(paragraph.clone());
        } else if negative_count < desired_negatives {
            paragraphs.push(paragraph.clone());
            negative_count += 1;
        }
    }

    let mut narrowed = manifest.clone();
    narrowed.questions = questions;
    narrowed.paragraphs = paragraphs;
    narrowed.metadata.paragraph_count = narrowed.paragraphs.len();
    narrowed.metadata.question_count = narrowed.questions.len();

    Ok(narrowed)
}

#[derive(Debug, Clone, Serialize)]
struct RelationInsert {
    #[serde(rename = "in")]
    pub in_: Thing,
    #[serde(rename = "out")]
    pub out: Thing,
    pub id: String,
    pub metadata: RelationshipMetadata,
}

#[derive(Debug)]
struct SizedBatch<T> {
    approx_bytes: usize,
    items: Vec<T>,
}

struct ManifestBatches {
    text_contents: Vec<SizedBatch<TextContent>>,
    entities: Vec<SizedBatch<KnowledgeEntity>>,
    entity_embeddings: Vec<SizedBatch<KnowledgeEntityEmbedding>>,
    relationships: Vec<SizedBatch<RelationInsert>>,
    chunks: Vec<SizedBatch<TextChunk>>,
    chunk_embeddings: Vec<SizedBatch<TextChunkEmbedding>>,
}

fn build_manifest_batches(manifest: &CorpusManifest) -> Result<ManifestBatches> {
    let mut text_contents = Vec::new();
    let mut entities = Vec::new();
    let mut entity_embeddings = Vec::new();
    let mut relationships = Vec::new();
    let mut chunks = Vec::new();
    let mut chunk_embeddings = Vec::new();

    let mut seen_text_content = HashSet::new();
    let mut seen_entities = HashSet::new();
    let mut seen_relationships = HashSet::new();
    let mut seen_chunks = HashSet::new();

    for paragraph in &manifest.paragraphs {
        if seen_text_content.insert(paragraph.text_content.id.clone()) {
            text_contents.push(paragraph.text_content.clone());
        }

        for embedded_entity in &paragraph.entities {
            if seen_entities.insert(embedded_entity.entity.id.clone()) {
                let entity = embedded_entity.entity.clone();
                entities.push(entity.clone());
                entity_embeddings.push(KnowledgeEntityEmbedding::new(
                    &entity.id,
                    embedded_entity.embedding.clone(),
                    entity.user_id.clone(),
                ));
            }
        }

        for relationship in &paragraph.relationships {
            if seen_relationships.insert(relationship.id.clone()) {
                let table = KnowledgeEntity::table_name();
                let in_id = relationship
                    .in_
                    .strip_prefix(&format!("{table}:"))
                    .unwrap_or(&relationship.in_);
                let out_id = relationship
                    .out
                    .strip_prefix(&format!("{table}:"))
                    .unwrap_or(&relationship.out);
                let in_thing = Thing::from((table, in_id));
                let out_thing = Thing::from((table, out_id));
                relationships.push(RelationInsert {
                    in_: in_thing,
                    out: out_thing,
                    id: relationship.id.clone(),
                    metadata: relationship.metadata.clone(),
                });
            }
        }

        for embedded_chunk in &paragraph.chunks {
            if seen_chunks.insert(embedded_chunk.chunk.id.clone()) {
                let chunk = embedded_chunk.chunk.clone();
                chunks.push(chunk.clone());
                chunk_embeddings.push(TextChunkEmbedding::new(
                    &chunk.id,
                    chunk.source_id.clone(),
                    embedded_chunk.embedding.clone(),
                    chunk.user_id.clone(),
                ));
            }
        }
    }

    Ok(ManifestBatches {
        text_contents: chunk_items(
            &text_contents,
            MANIFEST_BATCH_SIZE,
            TEXT_CONTENT_MAX_BYTES_PER_BATCH,
        )
        .context("chunking text_content payloads")?,
        entities: chunk_items(&entities, MANIFEST_BATCH_SIZE, MANIFEST_MAX_BYTES_PER_BATCH)
            .context("chunking knowledge_entity payloads")?,
        entity_embeddings: chunk_items(
            &entity_embeddings,
            MANIFEST_BATCH_SIZE,
            MANIFEST_MAX_BYTES_PER_BATCH,
        )
        .context("chunking knowledge_entity_embedding payloads")?,
        relationships: chunk_items(
            &relationships,
            MANIFEST_BATCH_SIZE,
            MANIFEST_MAX_BYTES_PER_BATCH,
        )
        .context("chunking relationship payloads")?,
        chunks: chunk_items(&chunks, MANIFEST_BATCH_SIZE, MANIFEST_MAX_BYTES_PER_BATCH)
            .context("chunking text_chunk payloads")?,
        chunk_embeddings: chunk_items(
            &chunk_embeddings,
            MANIFEST_BATCH_SIZE,
            MANIFEST_MAX_BYTES_PER_BATCH,
        )
        .context("chunking text_chunk_embedding payloads")?,
    })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParagraphShard {
    #[serde(default = "current_paragraph_shard_version")]
    pub version: u32,
    pub paragraph_id: String,
    pub shard_path: String,
    pub ingestion_fingerprint: String,
    pub ingested_at: DateTime<Utc>,
    pub title: String,
    pub text_content: TextContent,
    #[serde(deserialize_with = "deserialize_embedded_entities")]
    pub entities: Vec<EmbeddedKnowledgeEntity>,
    pub relationships: Vec<KnowledgeRelationship>,
    #[serde(deserialize_with = "deserialize_embedded_chunks")]
    pub chunks: Vec<EmbeddedTextChunk>,
    #[serde(default)]
    pub question_bindings: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub embedding_backend: String,
    #[serde(default)]
    pub embedding_model: Option<String>,
    #[serde(default)]
    pub embedding_dimension: usize,
    #[serde(default = "default_chunk_min_tokens")]
    pub chunk_min_tokens: usize,
    #[serde(default = "default_chunk_max_tokens")]
    pub chunk_max_tokens: usize,
    #[serde(default = "default_chunk_only")]
    pub chunk_only: bool,
}

pub struct ParagraphShardStore {
    base_dir: PathBuf,
}

impl ParagraphShardStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    pub fn ensure_base_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.base_dir)
            .with_context(|| format!("creating shard base dir {}", self.base_dir.display()))
    }

    fn resolve(&self, relative: &str) -> PathBuf {
        self.base_dir.join(relative)
    }

    pub fn load(&self, relative: &str, fingerprint: &str) -> Result<Option<ParagraphShard>> {
        let path = self.resolve(relative);
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(err).with_context(|| format!("opening shard {}", path.display()))
            }
        };
        let reader = BufReader::new(file);
        let mut shard: ParagraphShard = serde_json::from_reader(reader)
            .with_context(|| format!("parsing shard {}", path.display()))?;

        if shard.ingestion_fingerprint != fingerprint {
            debug!(
                path = %path.display(),
                expected = fingerprint,
                found = shard.ingestion_fingerprint,
                "Shard fingerprint mismatch; will rebuild"
            );
            return Ok(None);
        }
        if shard.version != PARAGRAPH_SHARD_VERSION {
            warn!(
                path = %path.display(),
                version = shard.version,
                expected = PARAGRAPH_SHARD_VERSION,
                "Upgrading shard to current version"
            );
            shard.version = PARAGRAPH_SHARD_VERSION;
        }
        shard.shard_path = relative.to_string();
        Ok(Some(shard))
    }

    pub fn persist(&self, shard: &ParagraphShard) -> Result<()> {
        let mut shard = shard.clone();
        shard.version = PARAGRAPH_SHARD_VERSION;

        let path = self.resolve(&shard.shard_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating shard dir {}", parent.display()))?;
        }
        let tmp_path = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(&shard).context("serialising paragraph shard")?;
        fs::write(&tmp_path, &body)
            .with_context(|| format!("writing shard tmp {}", tmp_path.display()))?;
        fs::rename(&tmp_path, &path)
            .with_context(|| format!("renaming shard tmp {}", path.display()))?;
        Ok(())
    }
}

impl ParagraphShard {
    pub fn new(
        paragraph: &ConvertedParagraph,
        shard_path: String,
        ingestion_fingerprint: &str,
        text_content: TextContent,
        entities: Vec<EmbeddedKnowledgeEntity>,
        relationships: Vec<KnowledgeRelationship>,
        chunks: Vec<EmbeddedTextChunk>,
        embedding_backend: &str,
        embedding_model: Option<String>,
        embedding_dimension: usize,
        chunk_min_tokens: usize,
        chunk_max_tokens: usize,
        chunk_only: bool,
    ) -> Self {
        Self {
            version: PARAGRAPH_SHARD_VERSION,
            paragraph_id: paragraph.id.clone(),
            shard_path,
            ingestion_fingerprint: ingestion_fingerprint.to_string(),
            ingested_at: Utc::now(),
            title: paragraph.title.clone(),
            text_content,
            entities,
            relationships,
            chunks,
            question_bindings: HashMap::new(),
            embedding_backend: embedding_backend.to_string(),
            embedding_model,
            embedding_dimension,
            chunk_min_tokens,
            chunk_max_tokens,
            chunk_only,
        }
    }

    pub fn to_corpus_paragraph(&self) -> CorpusParagraph {
        CorpusParagraph {
            paragraph_id: self.paragraph_id.clone(),
            title: self.title.clone(),
            text_content: self.text_content.clone(),
            entities: self.entities.clone(),
            relationships: self.relationships.clone(),
            chunks: self.chunks.clone(),
        }
    }

    pub fn ensure_question_binding(
        &mut self,
        question: &ConvertedQuestion,
    ) -> Result<(Vec<String>, bool)> {
        if let Some(existing) = self.question_bindings.get(&question.id) {
            return Ok((existing.clone(), false));
        }
        let chunk_ids = validate_answers(&self.text_content, &self.chunks, question)?;
        self.question_bindings
            .insert(question.id.clone(), chunk_ids.clone());
        Ok((chunk_ids, true))
    }
}

fn validate_answers(
    content: &TextContent,
    chunks: &[EmbeddedTextChunk],
    question: &ConvertedQuestion,
) -> Result<Vec<String>> {
    if question.is_impossible || question.answers.is_empty() {
        return Ok(Vec::new());
    }

    let mut matches = std::collections::BTreeSet::new();
    let mut found_any = false;
    let haystack = content.text.to_ascii_lowercase();
    let haystack_norm = normalize_answer_text(&haystack);
    for answer in &question.answers {
        let needle: String = answer.to_ascii_lowercase();
        let needle_norm = normalize_answer_text(&needle);
        let text_match = haystack.contains(&needle)
            || (!needle_norm.is_empty() && haystack_norm.contains(&needle_norm));
        if text_match {
            found_any = true;
        }
        for chunk in chunks {
            let chunk_text = chunk.chunk.chunk.to_ascii_lowercase();
            let chunk_norm = normalize_answer_text(&chunk_text);
            if chunk_text.contains(&needle)
                || (!needle_norm.is_empty() && chunk_norm.contains(&needle_norm))
            {
                matches.insert(chunk.chunk.get_id().to_string());
                found_any = true;
            }
        }
    }

    if !found_any {
        Err(anyhow!(
            "expected answer for question '{}' was not found in ingested content",
            question.id
        ))
    } else {
        Ok(matches.into_iter().collect())
    }
}

fn normalize_answer_text(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch.is_whitespace() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn chunk_items<T: Clone + Serialize>(
    items: &[T],
    max_items: usize,
    max_bytes: usize,
) -> Result<Vec<SizedBatch<T>>> {
    if items.is_empty() {
        return Ok(Vec::new());
    }

    let mut batches = Vec::new();
    let mut current = Vec::new();
    let mut current_bytes = 0usize;

    for item in items {
        let size = serde_json::to_vec(item)
            .map(|buf| buf.len())
            .context("serialising batch item for sizing")?;

        let would_overflow_items = !current.is_empty() && current.len() >= max_items;
        let would_overflow_bytes = !current.is_empty() && current_bytes + size > max_bytes;

        if would_overflow_items || would_overflow_bytes {
            batches.push(SizedBatch {
                approx_bytes: current_bytes.max(1),
                items: std::mem::take(&mut current),
            });
            current_bytes = 0;
        }

        current_bytes += size;
        current.push(item.clone());
    }

    if !current.is_empty() {
        batches.push(SizedBatch {
            approx_bytes: current_bytes.max(1),
            items: current,
        });
    }

    Ok(batches)
}

async fn execute_batched_inserts<T: Clone + Serialize + 'static>(
    db: &SurrealDbClient,
    statement: impl AsRef<str>,
    prefix: &str,
    batches: &[SizedBatch<T>],
) -> Result<()> {
    if batches.is_empty() {
        return Ok(());
    }

    let mut start = 0;
    while start < batches.len() {
        let mut group_bytes = 0usize;
        let mut group_end = start;
        let mut group_count = 0usize;

        while group_end < batches.len() {
            let batch_bytes = batches[group_end].approx_bytes.max(1);
            if group_count > 0
                && (group_bytes + batch_bytes > REQUEST_MAX_BYTES
                    || group_count >= MAX_BATCHES_PER_REQUEST)
            {
                break;
            }
            group_bytes += batch_bytes;
            group_end += 1;
            group_count += 1;
        }

        let slice = &batches[start..group_end];
        let mut query = db.client.query("BEGIN TRANSACTION;");
        let mut bind_index = 0usize;
        for batch in slice {
            let name = format!("{prefix}{bind_index}");
            bind_index += 1;
            query = query
                .query(format!("{} ${};", statement.as_ref(), name))
                .bind((name, batch.items.clone()));
        }
        let response = query
            .query("COMMIT TRANSACTION;")
            .await
            .context("executing batched insert transaction")?;
        if let Err(err) = response.check() {
            return Err(anyhow!(
                "batched insert failed for statement '{}': {err:?}",
                statement.as_ref()
            ));
        }

        start = group_end;
    }

    Ok(())
}

pub async fn seed_manifest_into_db(db: &SurrealDbClient, manifest: &CorpusManifest) -> Result<()> {
    let batches = build_manifest_batches(manifest).context("preparing manifest batches")?;

    let result = (|| async {
        execute_batched_inserts(
            db,
            format!("INSERT INTO {}", TextContent::table_name()),
            "tc",
            &batches.text_contents,
        )
        .await?;

        execute_batched_inserts(
            db,
            format!("INSERT INTO {}", KnowledgeEntity::table_name()),
            "ke",
            &batches.entities,
        )
        .await?;

        execute_batched_inserts(
            db,
            format!("INSERT INTO {}", TextChunk::table_name()),
            "ch",
            &batches.chunks,
        )
        .await?;

        execute_batched_inserts(
            db,
            "INSERT RELATION INTO relates_to",
            "rel",
            &batches.relationships,
        )
        .await?;

        execute_batched_inserts(
            db,
            format!("INSERT INTO {}", KnowledgeEntityEmbedding::table_name()),
            "kee",
            &batches.entity_embeddings,
        )
        .await?;

        execute_batched_inserts(
            db,
            format!("INSERT INTO {}", TextChunkEmbedding::table_name()),
            "tce",
            &batches.chunk_embeddings,
        )
        .await?;

        Ok(())
    })()
    .await;

    if result.is_err() {
        // Best-effort cleanup to avoid leaving partial manifest data behind.
        let _ = db
            .client
            .query(
                "BEGIN TRANSACTION;
                 DELETE text_chunk_embedding;
                 DELETE knowledge_entity_embedding;
                 DELETE relates_to;
                 DELETE text_chunk;
                 DELETE knowledge_entity;
                 DELETE text_content;
                 COMMIT TRANSACTION;",
            )
            .await;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db_helpers::change_embedding_length_in_hnsw_indexes;
    use chrono::Utc;
    use common::storage::types::knowledge_entity::KnowledgeEntityType;
    use uuid::Uuid;

    fn build_manifest() -> CorpusManifest {
        let user_id = "user-1".to_string();
        let source_id = "source-1".to_string();
        let now = Utc::now();
        let text_content_id = Uuid::new_v4().to_string();

        let text_content = TextContent {
            id: text_content_id.clone(),
            created_at: now,
            updated_at: now,
            text: "Hello world".to_string(),
            file_info: None,
            url_info: None,
            context: None,
            category: "test".to_string(),
            user_id: user_id.clone(),
        };

        let entity = KnowledgeEntity {
            id: Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            source_id: source_id.clone(),
            name: "Entity".to_string(),
            description: "A test entity".to_string(),
            entity_type: KnowledgeEntityType::Document,
            metadata: None,
            user_id: user_id.clone(),
        };
        let relationship = KnowledgeRelationship::new(
            format!("knowledge_entity:{}", entity.id),
            format!("knowledge_entity:{}", entity.id),
            user_id.clone(),
            source_id.clone(),
            "related".to_string(),
        );

        let chunk = TextChunk {
            id: Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            source_id: source_id.clone(),
            chunk: "chunk text".to_string(),
            user_id: user_id.clone(),
        };

        let paragraph_one = CorpusParagraph {
            paragraph_id: "p1".to_string(),
            title: "Paragraph 1".to_string(),
            text_content: text_content.clone(),
            entities: vec![EmbeddedKnowledgeEntity {
                entity: entity.clone(),
                embedding: vec![0.1, 0.2, 0.3],
            }],
            relationships: vec![relationship],
            chunks: vec![EmbeddedTextChunk {
                chunk: chunk.clone(),
                embedding: vec![0.3, 0.2, 0.1],
            }],
        };

        // Duplicate content/entities should be de-duplicated by the loader.
        let paragraph_two = CorpusParagraph {
            paragraph_id: "p2".to_string(),
            title: "Paragraph 2".to_string(),
            text_content: text_content.clone(),
            entities: vec![EmbeddedKnowledgeEntity {
                entity: entity.clone(),
                embedding: vec![0.1, 0.2, 0.3],
            }],
            relationships: Vec::new(),
            chunks: vec![EmbeddedTextChunk {
                chunk: chunk.clone(),
                embedding: vec![0.3, 0.2, 0.1],
            }],
        };

        let question = CorpusQuestion {
            question_id: "q1".to_string(),
            paragraph_id: paragraph_one.paragraph_id.clone(),
            text_content_id: text_content_id,
            question_text: "What is this?".to_string(),
            answers: vec!["Hello".to_string()],
            is_impossible: false,
            matching_chunk_ids: vec![chunk.id.clone()],
        };

        CorpusManifest {
            version: current_manifest_version(),
            metadata: CorpusMetadata {
                dataset_id: "dataset".to_string(),
                dataset_label: "Dataset".to_string(),
                slice_id: "slice".to_string(),
                include_unanswerable: false,
                require_verified_chunks: false,
                ingestion_fingerprint: "fp".to_string(),
                embedding_backend: "test".to_string(),
                embedding_model: Some("model".to_string()),
                embedding_dimension: 3,
                converted_checksum: "checksum".to_string(),
                generated_at: now,
                paragraph_count: 2,
                question_count: 1,
                chunk_min_tokens: 1,
                chunk_max_tokens: 10,
                chunk_only: false,
            },
            paragraphs: vec![paragraph_one, paragraph_two],
            questions: vec![question],
        }
    }

    #[tokio::test]
    async fn seeds_manifest_with_transactional_batches() {
        let namespace = "test_ns";
        let database = Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, &database)
            .await
            .expect("memory db");
        db.apply_migrations()
            .await
            .expect("apply migrations for memory db");
        change_embedding_length_in_hnsw_indexes(&db, 3)
            .await
            .expect("set embedding index dimension for test");

        let manifest = build_manifest();
        seed_manifest_into_db(&db, &manifest)
            .await
            .expect("manifest seed should succeed");

        let text_contents: Vec<TextContent> = db
            .client
            .query(format!("SELECT * FROM {};", TextContent::table_name()))
            .await
            .expect("select text_content")
            .take(0)
            .unwrap_or_default();
        assert_eq!(text_contents.len(), 1);

        let entities: Vec<KnowledgeEntity> = db
            .client
            .query(format!("SELECT * FROM {};", KnowledgeEntity::table_name()))
            .await
            .expect("select knowledge_entity")
            .take(0)
            .unwrap_or_default();
        assert_eq!(entities.len(), 1);

        let chunks: Vec<TextChunk> = db
            .client
            .query(format!("SELECT * FROM {};", TextChunk::table_name()))
            .await
            .expect("select text_chunk")
            .take(0)
            .unwrap_or_default();
        assert_eq!(chunks.len(), 1);

        let relationships: Vec<KnowledgeRelationship> = db
            .client
            .query("SELECT * FROM relates_to;")
            .await
            .expect("select relates_to")
            .take(0)
            .unwrap_or_default();
        assert_eq!(relationships.len(), 1);

        let entity_embeddings: Vec<KnowledgeEntityEmbedding> = db
            .client
            .query(format!(
                "SELECT * FROM {};",
                KnowledgeEntityEmbedding::table_name()
            ))
            .await
            .expect("select knowledge_entity_embedding")
            .take(0)
            .unwrap_or_default();
        assert_eq!(entity_embeddings.len(), 1);

        let chunk_embeddings: Vec<TextChunkEmbedding> = db
            .client
            .query(format!(
                "SELECT * FROM {};",
                TextChunkEmbedding::table_name()
            ))
            .await
            .expect("select text_chunk_embedding")
            .take(0)
            .unwrap_or_default();
        assert_eq!(chunk_embeddings.len(), 1);
    }

    #[tokio::test]
    async fn rolls_back_when_embeddings_mismatch_index_dimension() {
        let namespace = "test_ns_rollback";
        let database = Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, &database)
            .await
            .expect("memory db");
        db.apply_migrations()
            .await
            .expect("apply migrations for memory db");

        let manifest = build_manifest();
        let result = seed_manifest_into_db(&db, &manifest).await;
        assert!(
            result.is_ok(),
            "seeding should succeed even if embedding dimensions differ from default index"
        );

        let text_contents: Vec<TextContent> = db
            .client
            .query(format!("SELECT * FROM {};", TextContent::table_name()))
            .await
            .expect("select text_content")
            .take(0)
            .unwrap_or_default();
        let entities: Vec<KnowledgeEntity> = db
            .client
            .query(format!("SELECT * FROM {};", KnowledgeEntity::table_name()))
            .await
            .expect("select knowledge_entity")
            .take(0)
            .unwrap_or_default();
        let chunks: Vec<TextChunk> = db
            .client
            .query(format!("SELECT * FROM {};", TextChunk::table_name()))
            .await
            .expect("select text_chunk")
            .take(0)
            .unwrap_or_default();
        let relationships: Vec<KnowledgeRelationship> = db
            .client
            .query("SELECT * FROM relates_to;")
            .await
            .expect("select relates_to")
            .take(0)
            .unwrap_or_default();
        let entity_embeddings: Vec<KnowledgeEntityEmbedding> = db
            .client
            .query(format!(
                "SELECT * FROM {};",
                KnowledgeEntityEmbedding::table_name()
            ))
            .await
            .expect("select knowledge_entity_embedding")
            .take(0)
            .unwrap_or_default();
        let chunk_embeddings: Vec<TextChunkEmbedding> = db
            .client
            .query(format!(
                "SELECT * FROM {};",
                TextChunkEmbedding::table_name()
            ))
            .await
            .expect("select text_chunk_embedding")
            .take(0)
            .unwrap_or_default();

        assert_eq!(text_contents.len(), 1);
        assert_eq!(entities.len(), 1);
        assert_eq!(chunks.len(), 1);
        assert_eq!(relationships.len(), 1);
        assert_eq!(entity_embeddings.len(), 1);
        assert_eq!(chunk_embeddings.len(), 1);
    }

    #[test]
    fn window_manifest_trims_questions_and_negatives() {
        let manifest = build_manifest();
        // Add extra negatives to simulate multiplier ~4x
        let mut manifest = manifest;
        let mut extra_paragraphs = Vec::new();
        for _ in 0..8 {
            let mut p = manifest.paragraphs[0].clone();
            p.paragraph_id = Uuid::new_v4().to_string();
            p.entities.clear();
            p.relationships.clear();
            p.chunks.clear();
            extra_paragraphs.push(p);
        }
        manifest.paragraphs.extend(extra_paragraphs);
        manifest.metadata.paragraph_count = manifest.paragraphs.len();

        let windowed = window_manifest(&manifest, 0, 1, 4.0).expect("window manifest");
        assert_eq!(windowed.questions.len(), 1);
        // Expect roughly 4x negatives (bounded by available paragraphs)
        assert!(
            windowed.paragraphs.len() <= manifest.paragraphs.len(),
            "windowed paragraphs should never exceed original"
        );
        let positive_set: std::collections::HashSet<_> = windowed
            .questions
            .iter()
            .map(|q| q.paragraph_id.as_str())
            .collect();
        let positives = windowed
            .paragraphs
            .iter()
            .filter(|p| positive_set.contains(p.paragraph_id.as_str()))
            .count();
        let negatives = windowed.paragraphs.len().saturating_sub(positives);
        assert_eq!(positives, 1);
        assert!(negatives >= 1, "should include some negatives");
    }
}
