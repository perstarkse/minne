use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufReader, Read},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{anyhow, Context, Result};
use async_openai::Client;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::{
    storage::{
        db::SurrealDbClient,
        store::{DynStore, StorageManager},
        types::{
            ingestion_payload::IngestionPayload, ingestion_task::IngestionTask,
            knowledge_entity::KnowledgeEntity, knowledge_relationship::KnowledgeRelationship,
            text_chunk::TextChunk, text_content::TextContent,
        },
    },
    utils::config::{AppConfig, StorageKind},
};
use futures::future::try_join_all;
use ingestion_pipeline::{IngestionConfig, IngestionPipeline};
use object_store::memory::InMemory;
use sha2::{Digest, Sha256};
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    args::Config,
    datasets::{ConvertedDataset, ConvertedParagraph, ConvertedQuestion},
    embedding::EmbeddingProvider,
    slices::{self, ResolvedSlice, SliceParagraphKind},
};

const MANIFEST_VERSION: u32 = 1;
const INGESTION_SPEC_VERSION: u32 = 1;
const INGESTION_MAX_RETRIES: usize = 3;
const INGESTION_BATCH_SIZE: usize = 5;
const PARAGRAPH_SHARD_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct CorpusCacheConfig {
    pub ingestion_cache_dir: PathBuf,
    pub force_refresh: bool,
    pub refresh_embeddings_only: bool,
}

impl CorpusCacheConfig {
    pub fn new(
        ingestion_cache_dir: impl Into<PathBuf>,
        force_refresh: bool,
        refresh_embeddings_only: bool,
    ) -> Self {
        Self {
            ingestion_cache_dir: ingestion_cache_dir.into(),
            force_refresh,
            refresh_embeddings_only,
        }
    }
}

#[async_trait]
pub trait CorpusEmbeddingProvider: Send + Sync {
    fn backend_label(&self) -> &str;
    fn model_code(&self) -> Option<String>;
    fn dimension(&self) -> usize;
    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>>;
}

type OpenAIClient = Client<async_openai::config::OpenAIConfig>;

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
struct ParagraphShard {
    version: u32,
    paragraph_id: String,
    shard_path: String,
    ingestion_fingerprint: String,
    ingested_at: DateTime<Utc>,
    title: String,
    text_content: TextContent,
    entities: Vec<KnowledgeEntity>,
    relationships: Vec<KnowledgeRelationship>,
    chunks: Vec<TextChunk>,
    #[serde(default)]
    question_bindings: HashMap<String, Vec<String>>,
    #[serde(default)]
    embedding_backend: String,
    #[serde(default)]
    embedding_model: Option<String>,
    #[serde(default)]
    embedding_dimension: usize,
}

struct ParagraphShardStore {
    base_dir: PathBuf,
}

impl ParagraphShardStore {
    fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn ensure_base_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.base_dir)
            .with_context(|| format!("creating shard base dir {}", self.base_dir.display()))
    }

    fn resolve(&self, relative: &str) -> PathBuf {
        self.base_dir.join(relative)
    }

    fn load(&self, relative: &str, fingerprint: &str) -> Result<Option<ParagraphShard>> {
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

    fn persist(&self, shard: &ParagraphShard) -> Result<()> {
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

#[async_trait]
impl CorpusEmbeddingProvider for EmbeddingProvider {
    fn backend_label(&self) -> &str {
        EmbeddingProvider::backend_label(self)
    }

    fn model_code(&self) -> Option<String> {
        EmbeddingProvider::model_code(self)
    }

    fn dimension(&self) -> usize {
        EmbeddingProvider::dimension(self)
    }

    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        EmbeddingProvider::embed_batch(self, texts).await
    }
}

impl From<&Config> for CorpusCacheConfig {
    fn from(config: &Config) -> Self {
        CorpusCacheConfig::new(
            config.ingestion_cache_dir.clone(),
            config.force_convert || config.slice_reset_ingestion,
            config.refresh_embeddings_only,
        )
    }
}

impl ParagraphShard {
    fn new(
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

    fn to_corpus_paragraph(&self) -> CorpusParagraph {
        CorpusParagraph {
            paragraph_id: self.paragraph_id.clone(),
            title: self.title.clone(),
            text_content: self.text_content.clone(),
            entities: self.entities.clone(),
            relationships: self.relationships.clone(),
            chunks: self.chunks.clone(),
        }
    }

    fn ensure_question_binding(
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

#[derive(Clone)]
struct ParagraphShardRecord {
    shard: ParagraphShard,
    dirty: bool,
    needs_reembed: bool,
}

#[derive(Clone)]
struct IngestRequest<'a> {
    slot: usize,
    paragraph: &'a ConvertedParagraph,
    shard_path: String,
    question_refs: Vec<&'a ConvertedQuestion>,
}

impl<'a> IngestRequest<'a> {
    fn from_entry(
        slot: usize,
        paragraph: &'a ConvertedParagraph,
        entry: &'a slices::SliceParagraphEntry,
    ) -> Result<Self> {
        let shard_path = entry
            .shard_path
            .clone()
            .unwrap_or_else(|| slices::default_shard_path(&entry.id));
        let question_refs = match &entry.kind {
            SliceParagraphKind::Positive { question_ids } => question_ids
                .iter()
                .map(|id| {
                    paragraph
                        .questions
                        .iter()
                        .find(|question| question.id == *id)
                        .ok_or_else(|| {
                            anyhow!(
                                "paragraph '{}' missing question '{}' referenced by slice",
                                paragraph.id,
                                id
                            )
                        })
                })
                .collect::<Result<Vec<_>>>()?,
            SliceParagraphKind::Negative => Vec::new(),
        };
        Ok(Self {
            slot,
            paragraph,
            shard_path,
            question_refs,
        })
    }
}

struct ParagraphPlan<'a> {
    slot: usize,
    entry: &'a slices::SliceParagraphEntry,
    paragraph: &'a ConvertedParagraph,
}

#[derive(Default)]
struct IngestionStats {
    positive_reused: usize,
    positive_ingested: usize,
    negative_reused: usize,
    negative_ingested: usize,
}

pub async fn ensure_corpus<E: CorpusEmbeddingProvider>(
    dataset: &ConvertedDataset,
    slice: &ResolvedSlice<'_>,
    window: &slices::SliceWindow<'_>,
    cache: &CorpusCacheConfig,
    embedding: &E,
    openai: Arc<OpenAIClient>,
    user_id: &str,
    converted_path: &Path,
) -> Result<CorpusHandle> {
    let checksum = compute_file_checksum(converted_path)
        .with_context(|| format!("computing checksum for {}", converted_path.display()))?;
    let ingestion_fingerprint = build_ingestion_fingerprint(dataset, slice, &checksum);

    let base_dir = cache
        .ingestion_cache_dir
        .join(dataset.metadata.id.as_str())
        .join(slice.manifest.slice_id.as_str());
    if cache.force_refresh && !cache.refresh_embeddings_only {
        let _ = fs::remove_dir_all(&base_dir);
    }
    let store = ParagraphShardStore::new(base_dir.clone());
    store.ensure_base_dir()?;

    let positive_set: HashSet<&str> = window.positive_ids().collect();
    let embedding_backend_label = embedding.backend_label().to_string();
    let embedding_model_code = embedding.model_code();
    let embedding_dimension = embedding.dimension();
    if positive_set.is_empty() {
        return Err(anyhow!(
            "window selection contains zero positive paragraphs for slice '{}'",
            slice.manifest.slice_id
        ));
    }

    let mut plan = Vec::new();
    for (idx, entry) in slice.manifest.paragraphs.iter().enumerate() {
        let include = match &entry.kind {
            SliceParagraphKind::Positive { .. } => positive_set.contains(entry.id.as_str()),
            SliceParagraphKind::Negative => true,
        };
        if include {
            let paragraph = slice
                .paragraphs
                .get(idx)
                .copied()
                .ok_or_else(|| anyhow!("slice missing paragraph index {}", idx))?;
            plan.push(ParagraphPlan {
                slot: plan.len(),
                entry,
                paragraph,
            });
        }
    }

    if plan.is_empty() {
        return Err(anyhow!(
            "no paragraphs selected for ingestion (slice '{}')",
            slice.manifest.slice_id
        ));
    }

    let mut records: Vec<Option<ParagraphShardRecord>> = vec![None; plan.len()];
    let mut ingest_requests = Vec::new();
    let mut stats = IngestionStats::default();

    for plan_entry in &plan {
        let shard_path = plan_entry
            .entry
            .shard_path
            .clone()
            .unwrap_or_else(|| slices::default_shard_path(&plan_entry.entry.id));
        let shard = if cache.force_refresh {
            None
        } else {
            store.load(&shard_path, &ingestion_fingerprint)?
        };
        if let Some(shard) = shard {
            let model_matches = shard.embedding_model.as_deref() == embedding_model_code.as_deref();
            let needs_reembed = shard.embedding_backend != embedding_backend_label
                || shard.embedding_dimension != embedding_dimension
                || !model_matches;
            match plan_entry.entry.kind {
                SliceParagraphKind::Positive { .. } => stats.positive_reused += 1,
                SliceParagraphKind::Negative => stats.negative_reused += 1,
            }
            records[plan_entry.slot] = Some(ParagraphShardRecord {
                shard,
                dirty: false,
                needs_reembed,
            });
        } else {
            match plan_entry.entry.kind {
                SliceParagraphKind::Positive { .. } => stats.positive_ingested += 1,
                SliceParagraphKind::Negative => stats.negative_ingested += 1,
            }
            let request =
                IngestRequest::from_entry(plan_entry.slot, plan_entry.paragraph, plan_entry.entry)?;
            ingest_requests.push(request);
        }
    }

    if cache.refresh_embeddings_only && !ingest_requests.is_empty() {
        return Err(anyhow!(
            "--refresh-embeddings requested but {} shard(s) missing for dataset '{}' slice '{}'",
            ingest_requests.len(),
            dataset.metadata.id,
            slice.manifest.slice_id
        ));
    }

    if !ingest_requests.is_empty() {
        let new_shards = ingest_paragraph_batch(
            dataset,
            &ingest_requests,
            embedding,
            openai.clone(),
            user_id,
            &ingestion_fingerprint,
            &embedding_backend_label,
            embedding_model_code.clone(),
            embedding_dimension,
        )
        .await
        .context("ingesting missing slice paragraphs")?;
        for (request, shard) in ingest_requests.into_iter().zip(new_shards.into_iter()) {
            store.persist(&shard)?;
            records[request.slot] = Some(ParagraphShardRecord {
                shard,
                dirty: false,
                needs_reembed: false,
            });
        }
    }

    for record in &mut records {
        let shard_record = record
            .as_mut()
            .context("shard record missing after ingestion run")?;
        if cache.refresh_embeddings_only || shard_record.needs_reembed {
            reembed_entities(&mut shard_record.shard.entities, embedding).await?;
            reembed_chunks(&mut shard_record.shard.chunks, embedding).await?;
            shard_record.shard.ingestion_fingerprint = ingestion_fingerprint.clone();
            shard_record.shard.ingested_at = Utc::now();
            shard_record.shard.embedding_backend = embedding_backend_label.clone();
            shard_record.shard.embedding_model = embedding_model_code.clone();
            shard_record.shard.embedding_dimension = embedding_dimension;
            shard_record.dirty = true;
            shard_record.needs_reembed = false;
        }
    }

    let mut record_index = HashMap::new();
    for (idx, plan_entry) in plan.iter().enumerate() {
        record_index.insert(plan_entry.entry.id.as_str(), idx);
    }

    let mut corpus_paragraphs = Vec::with_capacity(plan.len());
    for record in &records {
        let shard = &record.as_ref().expect("record missing").shard;
        corpus_paragraphs.push(shard.to_corpus_paragraph());
    }

    let mut corpus_questions = Vec::with_capacity(window.cases.len());
    for case in &window.cases {
        let slot = record_index
            .get(case.paragraph.id.as_str())
            .copied()
            .ok_or_else(|| {
                anyhow!(
                    "slice case references paragraph '{}' that is not part of the window",
                    case.paragraph.id
                )
            })?;
        let record_slot = records
            .get_mut(slot)
            .context("shard record slot missing for question binding")?;
        let record = record_slot
            .as_mut()
            .context("shard record missing for question binding")?;
        let (chunk_ids, updated) = match record.shard.ensure_question_binding(case.question) {
            Ok(result) => result,
            Err(err) => {
                warn!(
                    question_id = %case.question.id,
                    paragraph_id = %case.paragraph.id,
                    error = %err,
                    "Failed to locate answer text in ingested content; recording empty chunk bindings"
                );
                record
                    .shard
                    .question_bindings
                    .insert(case.question.id.clone(), Vec::new());
                record.dirty = true;
                (Vec::new(), true)
            }
        };
        if updated {
            record.dirty = true;
        }
        corpus_questions.push(CorpusQuestion {
            question_id: case.question.id.clone(),
            paragraph_id: case.paragraph.id.clone(),
            text_content_id: record.shard.text_content.id.clone(),
            question_text: case.question.question.clone(),
            answers: case.question.answers.clone(),
            is_impossible: case.question.is_impossible,
            matching_chunk_ids: chunk_ids,
        });
    }

    for record in &mut records {
        if let Some(ref mut entry) = record {
            if entry.dirty {
                store.persist(&entry.shard)?;
            }
        }
    }

    let manifest = CorpusManifest {
        version: MANIFEST_VERSION,
        metadata: CorpusMetadata {
            dataset_id: dataset.metadata.id.clone(),
            dataset_label: dataset.metadata.label.clone(),
            slice_id: slice.manifest.slice_id.clone(),
            include_unanswerable: slice.manifest.includes_unanswerable,
            ingestion_fingerprint: ingestion_fingerprint.clone(),
            embedding_backend: embedding.backend_label().to_string(),
            embedding_model: embedding.model_code(),
            embedding_dimension: embedding.dimension(),
            converted_checksum: checksum,
            generated_at: Utc::now(),
            paragraph_count: corpus_paragraphs.len(),
            question_count: corpus_questions.len(),
        },
        paragraphs: corpus_paragraphs,
        questions: corpus_questions,
    };

    let ingested_count = stats.positive_ingested + stats.negative_ingested;
    let reused_ingestion = ingested_count == 0 && !cache.force_refresh;
    let reused_embeddings = reused_ingestion && !cache.refresh_embeddings_only;

    Ok(CorpusHandle {
        manifest,
        path: base_dir,
        reused_ingestion,
        reused_embeddings,
        positive_reused: stats.positive_reused,
        positive_ingested: stats.positive_ingested,
        negative_reused: stats.negative_reused,
        negative_ingested: stats.negative_ingested,
    })
}

async fn reembed_entities<E: CorpusEmbeddingProvider>(
    entities: &mut [KnowledgeEntity],
    embedding: &E,
) -> Result<()> {
    if entities.is_empty() {
        return Ok(());
    }
    let payloads: Vec<String> = entities.iter().map(entity_embedding_text).collect();
    let vectors = embedding.embed_batch(payloads).await?;
    if vectors.len() != entities.len() {
        return Err(anyhow!(
            "entity embedding batch mismatch (expected {}, got {})",
            entities.len(),
            vectors.len()
        ));
    }
    for (entity, vector) in entities.iter_mut().zip(vectors.into_iter()) {
        entity.embedding = vector;
    }
    Ok(())
}

async fn reembed_chunks<E: CorpusEmbeddingProvider>(
    chunks: &mut [TextChunk],
    embedding: &E,
) -> Result<()> {
    if chunks.is_empty() {
        return Ok(());
    }
    let payloads: Vec<String> = chunks.iter().map(|chunk| chunk.chunk.clone()).collect();
    let vectors = embedding.embed_batch(payloads).await?;
    if vectors.len() != chunks.len() {
        return Err(anyhow!(
            "chunk embedding batch mismatch (expected {}, got {})",
            chunks.len(),
            vectors.len()
        ));
    }
    for (chunk, vector) in chunks.iter_mut().zip(vectors.into_iter()) {
        chunk.embedding = vector;
    }
    Ok(())
}

fn entity_embedding_text(entity: &KnowledgeEntity) -> String {
    format!(
        "name: {}\ndescription: {}\ntype: {:?}",
        entity.name, entity.description, entity.entity_type
    )
}

async fn ingest_paragraph_batch<E: CorpusEmbeddingProvider>(
    dataset: &ConvertedDataset,
    targets: &[IngestRequest<'_>],
    embedding: &E,
    openai: Arc<OpenAIClient>,
    user_id: &str,
    ingestion_fingerprint: &str,
    embedding_backend: &str,
    embedding_model: Option<String>,
    embedding_dimension: usize,
) -> Result<Vec<ParagraphShard>> {
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    let namespace = format!("ingest_eval_{}", Uuid::new_v4());
    let db = Arc::new(
        SurrealDbClient::memory(&namespace, "corpus")
            .await
            .context("creating ingestion SurrealDB instance")?,
    );
    db.apply_migrations()
        .await
        .context("applying migrations for ingestion")?;

    let mut app_config = AppConfig::default();
    app_config.storage = StorageKind::Memory;
    let backend: DynStore = Arc::new(InMemory::new());
    let storage = StorageManager::with_backend(backend, StorageKind::Memory);

    let pipeline = IngestionPipeline::new(
        db,
        openai.clone(),
        app_config,
        None::<Arc<composite_retrieval::reranking::RerankerPool>>,
        storage,
    )
    .await?;
    let pipeline = Arc::new(pipeline);

    let mut shards = Vec::with_capacity(targets.len());
    let category = dataset.metadata.category.clone();
    for (batch_index, batch) in targets.chunks(INGESTION_BATCH_SIZE).enumerate() {
        info!(
            batch = batch_index,
            batch_size = batch.len(),
            total_batches = (targets.len() + INGESTION_BATCH_SIZE - 1) / INGESTION_BATCH_SIZE,
            "Ingesting paragraph batch"
        );
        let model_clone = embedding_model.clone();
        let backend_clone = embedding_backend.to_string();
        let pipeline_clone = pipeline.clone();
        let category_clone = category.clone();
        let tasks = batch.iter().cloned().map(move |request| {
            ingest_single_paragraph(
                pipeline_clone.clone(),
                request,
                category_clone.clone(),
                embedding,
                user_id,
                ingestion_fingerprint,
                backend_clone.clone(),
                model_clone.clone(),
                embedding_dimension,
            )
        });
        let batch_results: Vec<ParagraphShard> = try_join_all(tasks)
            .await
            .context("ingesting batch of paragraphs")?;
        shards.extend(batch_results);
    }

    Ok(shards)
}

async fn ingest_single_paragraph<E: CorpusEmbeddingProvider>(
    pipeline: Arc<IngestionPipeline>,
    request: IngestRequest<'_>,
    category: String,
    embedding: &E,
    user_id: &str,
    ingestion_fingerprint: &str,
    embedding_backend: String,
    embedding_model: Option<String>,
    embedding_dimension: usize,
) -> Result<ParagraphShard> {
    let paragraph = request.paragraph;
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 1..=INGESTION_MAX_RETRIES {
        let payload = IngestionPayload::Text {
            text: paragraph.context.clone(),
            context: paragraph.title.clone(),
            category: category.clone(),
            user_id: user_id.to_string(),
        };
        let task = IngestionTask::new(payload, user_id.to_string());
        match pipeline.produce_artifacts(&task).await {
            Ok(mut artifacts) => {
                reembed_entities(&mut artifacts.entities, embedding).await?;
                reembed_chunks(&mut artifacts.chunks, embedding).await?;
                let mut shard = ParagraphShard::new(
                    paragraph,
                    request.shard_path,
                    ingestion_fingerprint,
                    artifacts.text_content,
                    artifacts.entities,
                    artifacts.relationships,
                    artifacts.chunks,
                    &embedding_backend,
                    embedding_model.clone(),
                    embedding_dimension,
                );
                for question in &request.question_refs {
                    if let Err(err) = shard.ensure_question_binding(question) {
                        warn!(
                            question_id = %question.id,
                            paragraph_id = %paragraph.id,
                            error = %err,
                            "Failed to locate answer text in ingested content; recording empty chunk bindings"
                        );
                        shard
                            .question_bindings
                            .insert(question.id.clone(), Vec::new());
                    }
                }
                return Ok(shard);
            }
            Err(err) => {
                warn!(
                    paragraph_id = %paragraph.id,
                    attempt,
                    max_attempts = INGESTION_MAX_RETRIES,
                    error = ?err,
                    "ingestion attempt failed for paragraph; retrying"
                );
                last_err = Some(err.into());
            }
        }
    }

    Err(last_err
        .unwrap_or_else(|| anyhow!("ingestion failed"))
        .context(format!("running ingestion for paragraph {}", paragraph.id)))
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

fn build_ingestion_fingerprint(
    dataset: &ConvertedDataset,
    slice: &ResolvedSlice<'_>,
    checksum: &str,
) -> String {
    let config_repr = format!("{:?}", IngestionConfig::default());
    let mut hasher = Sha256::new();
    hasher.update(config_repr.as_bytes());
    let config_hash = format!("{:x}", hasher.finalize());

    format!(
        "v{INGESTION_SPEC_VERSION}:{}:{}:{}:{}:{}",
        dataset.metadata.id,
        slice.manifest.slice_id,
        slice.manifest.includes_unanswerable,
        checksum,
        config_hash
    )
}

fn compute_file_checksum(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("opening file {} for checksum", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("reading {} for checksum", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub async fn seed_manifest_into_db(db: &SurrealDbClient, manifest: &CorpusManifest) -> Result<()> {
    for paragraph in &manifest.paragraphs {
        db.store_item(paragraph.text_content.clone())
            .await
            .context("storing text_content from manifest")?;
        for entity in &paragraph.entities {
            db.store_item(entity.clone())
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
            db.store_item(chunk.clone())
                .await
                .context("storing text_chunk from manifest")?;
        }
    }

    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datasets::ConvertedQuestion;

    fn mock_text_content() -> TextContent {
        TextContent {
            id: "tc1".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            text: "alpha beta gamma".into(),
            file_info: None,
            url_info: None,
            context: Some("ctx".into()),
            category: "cat".into(),
            user_id: "user".into(),
        }
    }

    fn mock_chunk(id: &str, text: &str) -> TextChunk {
        TextChunk {
            id: id.into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            source_id: "src".into(),
            chunk: text.into(),
            embedding: vec![],
            user_id: "user".into(),
        }
    }

    #[test]
    fn validate_answers_passes_when_present() {
        let content = mock_text_content();
        let chunk = mock_chunk("chunk1", "alpha chunk");
        let question = ConvertedQuestion {
            id: "q1".into(),
            question: "?".into(),
            answers: vec!["Alpha".into()],
            is_impossible: false,
        };
        let matches = validate_answers(&content, &[chunk], &question).expect("answers match");
        assert_eq!(matches, vec!["chunk1".to_string()]);
    }

    #[test]
    fn validate_answers_fails_when_missing() {
        let question = ConvertedQuestion {
            id: "q1".into(),
            question: "?".into(),
            answers: vec!["delta".into()],
            is_impossible: false,
        };
        let err = validate_answers(
            &mock_text_content(),
            &[mock_chunk("chunk", "alpha")],
            &question,
        )
        .expect_err("missing answer should fail");
        assert!(err.to_string().contains("not found"));
    }
}
