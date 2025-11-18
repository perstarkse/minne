use std::{collections::HashMap, fs, io::BufReader, path::PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use common::storage::{
    db::SurrealDbClient,
    types::{
        knowledge_entity::KnowledgeEntity, knowledge_relationship::KnowledgeRelationship,
        text_chunk::TextChunk, text_content::TextContent,
    },
};
use tracing::warn;

use crate::datasets::{ConvertedParagraph, ConvertedQuestion};

pub const MANIFEST_VERSION: u32 = 1;
pub const PARAGRAPH_SHARD_VERSION: u32 = 1;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CorpusManifest {
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
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CorpusParagraph {
    pub paragraph_id: String,
    pub title: String,
    pub text_content: TextContent,
    pub entities: Vec<KnowledgeEntity>,
    pub relationships: Vec<KnowledgeRelationship>,
    pub chunks: Vec<TextChunk>,
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParagraphShard {
    pub version: u32,
    pub paragraph_id: String,
    pub shard_path: String,
    pub ingestion_fingerprint: String,
    pub ingested_at: DateTime<Utc>,
    pub title: String,
    pub text_content: TextContent,
    pub entities: Vec<KnowledgeEntity>,
    pub relationships: Vec<KnowledgeRelationship>,
    pub chunks: Vec<TextChunk>,
    #[serde(default)]
    pub question_bindings: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub embedding_backend: String,
    #[serde(default)]
    pub embedding_model: Option<String>,
    #[serde(default)]
    pub embedding_dimension: usize,
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
        if shard.version != PARAGRAPH_SHARD_VERSION {
            warn!(
                path = %path.display(),
                version = shard.version,
                expected = PARAGRAPH_SHARD_VERSION,
                "Skipping shard due to version mismatch"
            );
            return Ok(None);
        }
        if shard.ingestion_fingerprint != fingerprint {
            return Ok(None);
        }
        shard.shard_path = relative.to_string();
        Ok(Some(shard))
    }

    pub fn persist(&self, shard: &ParagraphShard) -> Result<()> {
        let path = self.resolve(&shard.shard_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating shard dir {}", parent.display()))?;
        }
        let tmp_path = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(shard).context("serialising paragraph shard")?;
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
        entities: Vec<KnowledgeEntity>,
        relationships: Vec<KnowledgeRelationship>,
        chunks: Vec<TextChunk>,
        embedding_backend: &str,
        embedding_model: Option<String>,
        embedding_dimension: usize,
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
    chunks: &[TextChunk],
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
            let chunk_text = chunk.chunk.to_ascii_lowercase();
            let chunk_norm = normalize_answer_text(&chunk_text);
            if chunk_text.contains(&needle)
                || (!needle_norm.is_empty() && chunk_norm.contains(&needle_norm))
            {
                matches.insert(chunk.id.clone());
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

pub async fn seed_manifest_into_db(db: &SurrealDbClient, manifest: &CorpusManifest) -> Result<()> {
    for paragraph in &manifest.paragraphs {
        db.upsert_item(paragraph.text_content.clone())
            .await
            .context("storing text_content from manifest")?;
        for entity in &paragraph.entities {
            db.upsert_item(entity.clone())
                .await
                .context("storing knowledge_entity from manifest")?;
        }
        for relationship in &paragraph.relationships {
            relationship
                .store_relationship(db)
                .await
                .context("storing knowledge_relationship from manifest")?;
        }
        for chunk in &paragraph.chunks {
            db.upsert_item(chunk.clone())
                .await
                .context("storing text_chunk from manifest")?;
        }
    }

    Ok(())
}
