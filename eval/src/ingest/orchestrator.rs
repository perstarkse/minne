use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{anyhow, Context, Result};
use async_openai::Client;
use chrono::Utc;
use common::{
    storage::{
        db::SurrealDbClient,
        store::{DynStore, StorageManager},
        types::{ingestion_payload::IngestionPayload, ingestion_task::IngestionTask, StoredObject},
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
    datasets::{ConvertedDataset, ConvertedParagraph, ConvertedQuestion},
    db_helpers::change_embedding_length_in_hnsw_indexes,
    slices::{self, ResolvedSlice, SliceParagraphKind},
};

use crate::ingest::{
    CorpusCacheConfig, CorpusHandle, CorpusManifest, CorpusMetadata, CorpusQuestion,
    EmbeddedKnowledgeEntity, EmbeddedTextChunk, ParagraphShard, ParagraphShardStore,
    MANIFEST_VERSION,
};

const INGESTION_SPEC_VERSION: u32 = 2;

type OpenAIClient = Client<async_openai::config::OpenAIConfig>;

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

pub async fn ensure_corpus(
    dataset: &ConvertedDataset,
    slice: &ResolvedSlice<'_>,
    window: &slices::SliceWindow<'_>,
    cache: &CorpusCacheConfig,
    embedding: Arc<common::utils::embedding::EmbeddingProvider>,
    openai: Arc<OpenAIClient>,
    user_id: &str,
    converted_path: &Path,
    ingestion_config: IngestionConfig,
) -> Result<CorpusHandle> {
    let checksum = compute_file_checksum(converted_path)
        .with_context(|| format!("computing checksum for {}", converted_path.display()))?;
    let ingestion_fingerprint =
        build_ingestion_fingerprint(dataset, slice, &checksum, &ingestion_config);

    let base_dir = cached_corpus_dir(
        cache,
        dataset.metadata.id.as_str(),
        slice.manifest.slice_id.as_str(),
    );
    if cache.force_refresh && !cache.refresh_embeddings_only {
        let _ = fs::remove_dir_all(&base_dir);
    }
    let store = ParagraphShardStore::new(base_dir.clone());
    store.ensure_base_dir()?;

    let positive_set: HashSet<&str> = window.positive_ids().collect();
    let require_verified_chunks = slice.manifest.require_verified_chunks;
    let embedding_backend_label = embedding.backend_label().to_string();
    let embedding_model_code = embedding.model_code();
    let embedding_dimension = embedding.dimension();
    if positive_set.is_empty() {
        return Err(anyhow!(
            "window selection contains zero positive paragraphs for slice '{}'",
            slice.manifest.slice_id
        ));
    }

    let desired_negatives =
        ((positive_set.len() as f32) * slice.manifest.negative_multiplier).ceil() as usize;
    let mut plan = Vec::new();
    let mut negatives_added = 0usize;
    for (idx, entry) in slice.manifest.paragraphs.iter().enumerate() {
        let include = match &entry.kind {
            SliceParagraphKind::Positive { .. } => positive_set.contains(entry.id.as_str()),
            SliceParagraphKind::Negative => {
                negatives_added < desired_negatives && {
                    negatives_added += 1;
                    true
                }
            }
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
            embedding.clone(),
            openai.clone(),
            user_id,
            &ingestion_fingerprint,
            &embedding_backend_label,
            embedding_model_code.clone(),
            embedding_dimension,
            cache.ingestion_batch_size,
            cache.ingestion_max_retries,
            ingestion_config.clone(),
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
            // Embeddings are now generated by the pipeline using FastEmbed - no need to re-embed
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
                if require_verified_chunks {
                    return Err(err).context(format!(
                        "locating answer text for question '{}' in paragraph '{}'",
                        case.question.id, case.paragraph.id
                    ));
                }
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
            text_content_id: record.shard.text_content.get_id().to_string(),
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
            require_verified_chunks: slice.manifest.require_verified_chunks,
            ingestion_fingerprint: ingestion_fingerprint.clone(),
            embedding_backend: embedding.backend_label().to_string(),
            embedding_model: embedding.model_code(),
            embedding_dimension: embedding.dimension(),
            converted_checksum: checksum,
            generated_at: Utc::now(),
            paragraph_count: corpus_paragraphs.len(),
            question_count: corpus_questions.len(),
            chunk_min_tokens: ingestion_config.tuning.chunk_min_tokens,
            chunk_max_tokens: ingestion_config.tuning.chunk_max_tokens,
            chunk_only: ingestion_config.chunk_only,
        },
        paragraphs: corpus_paragraphs,
        questions: corpus_questions,
    };

    let ingested_count = stats.positive_ingested + stats.negative_ingested;
    let reused_ingestion = ingested_count == 0 && !cache.force_refresh;
    let reused_embeddings = reused_ingestion && !cache.refresh_embeddings_only;

    let handle = CorpusHandle {
        manifest,
        path: base_dir,
        reused_ingestion,
        reused_embeddings,
        positive_reused: stats.positive_reused,
        positive_ingested: stats.positive_ingested,
        negative_reused: stats.negative_reused,
        negative_ingested: stats.negative_ingested,
    };

    persist_manifest(&handle).context("persisting corpus manifest")?;

    Ok(handle)
}

async fn ingest_paragraph_batch(
    dataset: &ConvertedDataset,
    targets: &[IngestRequest<'_>],
    embedding: Arc<common::utils::embedding::EmbeddingProvider>,
    openai: Arc<OpenAIClient>,
    user_id: &str,
    ingestion_fingerprint: &str,
    embedding_backend: &str,
    embedding_model: Option<String>,
    embedding_dimension: usize,
    batch_size: usize,
    max_retries: usize,
    ingestion_config: IngestionConfig,
) -> Result<Vec<ParagraphShard>> {
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    let namespace = format!("ingest_eval_{}", Uuid::new_v4());
    let db = Arc::new(
        SurrealDbClient::memory(&namespace, "corpus")
            .await
            .context("creating in-memory surrealdb for ingestion")?,
    );
    db.apply_migrations()
        .await
        .context("applying migrations for ingestion")?;

    change_embedding_length_in_hnsw_indexes(&db, embedding_dimension)
        .await
        .context("failed setting new hnsw length")?;

    let mut app_config = AppConfig::default();
    app_config.storage = StorageKind::Memory;
    let backend: DynStore = Arc::new(InMemory::new());
    let storage = StorageManager::with_backend(backend, StorageKind::Memory);

    let pipeline_config = ingestion_config.clone();
    let pipeline = IngestionPipeline::new_with_config(
        db,
        openai.clone(),
        app_config,
        None::<Arc<retrieval_pipeline::reranking::RerankerPool>>,
        storage,
        embedding.clone(),
        pipeline_config,
    )
    .await?;
    let pipeline = Arc::new(pipeline);

    let mut shards = Vec::with_capacity(targets.len());
    let category = dataset.metadata.category.clone();
    for (batch_index, batch) in targets.chunks(batch_size).enumerate() {
        info!(
            batch = batch_index,
            batch_size = batch.len(),
            total_batches = (targets.len() + batch_size - 1) / batch_size,
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
                user_id,
                ingestion_fingerprint,
                backend_clone.clone(),
                model_clone.clone(),
                embedding_dimension,
                max_retries,
                ingestion_config.tuning.chunk_min_tokens,
                ingestion_config.tuning.chunk_max_tokens,
                ingestion_config.chunk_only,
            )
        });
        let batch_results: Vec<ParagraphShard> = try_join_all(tasks)
            .await
            .context("ingesting batch of paragraphs")?;
        shards.extend(batch_results);
    }

    Ok(shards)
}

async fn ingest_single_paragraph(
    pipeline: Arc<IngestionPipeline>,
    request: IngestRequest<'_>,
    category: String,
    user_id: &str,
    ingestion_fingerprint: &str,
    embedding_backend: String,
    embedding_model: Option<String>,
    embedding_dimension: usize,
    max_retries: usize,
    chunk_min_tokens: usize,
    chunk_max_tokens: usize,
    chunk_only: bool,
) -> Result<ParagraphShard> {
    let paragraph = request.paragraph;
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 1..=max_retries {
        let payload = IngestionPayload::Text {
            text: paragraph.context.clone(),
            context: paragraph.title.clone(),
            category: category.clone(),
            user_id: user_id.to_string(),
        };
        let task = IngestionTask::new(payload, user_id.to_string());
        match pipeline.produce_artifacts(&task).await {
            Ok(artifacts) => {
                let entities: Vec<EmbeddedKnowledgeEntity> = artifacts
                    .entities
                    .into_iter()
                    .map(|e| EmbeddedKnowledgeEntity {
                        entity: e.entity,
                        embedding: e.embedding,
                    })
                    .collect();
                let chunks: Vec<EmbeddedTextChunk> = artifacts
                    .chunks
                    .into_iter()
                    .map(|c| EmbeddedTextChunk {
                        chunk: c.chunk,
                        embedding: c.embedding,
                    })
                    .collect();
                // No need to reembed - pipeline now uses FastEmbed internally
                let mut shard = ParagraphShard::new(
                    paragraph,
                    request.shard_path,
                    ingestion_fingerprint,
                    artifacts.text_content,
                    entities,
                    artifacts.relationships,
                    chunks,
                    &embedding_backend,
                    embedding_model.clone(),
                    embedding_dimension,
                    chunk_min_tokens,
                    chunk_max_tokens,
                    chunk_only,
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
                    max_attempts = max_retries,
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

pub fn cached_corpus_dir(cache: &CorpusCacheConfig, dataset_id: &str, slice_id: &str) -> PathBuf {
    cache.ingestion_cache_dir.join(dataset_id).join(slice_id)
}

pub fn build_ingestion_fingerprint(
    dataset: &ConvertedDataset,
    slice: &ResolvedSlice<'_>,
    checksum: &str,
    ingestion_config: &IngestionConfig,
) -> String {
    let config_repr = format!("{:?}", ingestion_config);
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

pub fn compute_ingestion_fingerprint(
    dataset: &ConvertedDataset,
    slice: &ResolvedSlice<'_>,
    converted_path: &Path,
    ingestion_config: &IngestionConfig,
) -> Result<String> {
    let checksum = compute_file_checksum(converted_path)?;
    Ok(build_ingestion_fingerprint(
        dataset,
        slice,
        &checksum,
        ingestion_config,
    ))
}

pub fn load_cached_manifest(base_dir: &Path) -> Result<Option<CorpusManifest>> {
    let path = base_dir.join("manifest.json");
    if !path.exists() {
        return Ok(None);
    }
    let mut file = fs::File::open(&path)
        .with_context(|| format!("opening cached manifest {}", path.display()))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .with_context(|| format!("reading cached manifest {}", path.display()))?;
    let manifest: CorpusManifest = serde_json::from_slice(&buf)
        .with_context(|| format!("deserialising cached manifest {}", path.display()))?;
    Ok(Some(manifest))
}

fn persist_manifest(handle: &CorpusHandle) -> Result<()> {
    let path = handle.path.join("manifest.json");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating manifest directory {}", parent.display()))?;
    }
    let tmp_path = path.with_extension("json.tmp");
    let blob =
        serde_json::to_vec_pretty(&handle.manifest).context("serialising corpus manifest")?;
    fs::write(&tmp_path, &blob)
        .with_context(|| format!("writing temporary manifest {}", tmp_path.display()))?;
    fs::rename(&tmp_path, &path)
        .with_context(|| format!("replacing manifest {}", path.display()))?;
    Ok(())
}

pub fn corpus_handle_from_manifest(manifest: CorpusManifest, base_dir: PathBuf) -> CorpusHandle {
    CorpusHandle {
        manifest,
        path: base_dir,
        reused_ingestion: true,
        reused_embeddings: true,
        positive_reused: 0,
        positive_ingested: 0,
        negative_reused: 0,
        negative_ingested: 0,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        datasets::{ConvertedDataset, ConvertedParagraph, ConvertedQuestion, DatasetKind},
        slices::{CaseRef, SliceCaseEntry, SliceManifest, SliceParagraphEntry, SliceParagraphKind},
    };
    use chrono::Utc;

    fn dummy_dataset() -> ConvertedDataset {
        let question = ConvertedQuestion {
            id: "q1".to_string(),
            question: "What?".to_string(),
            answers: vec!["A".to_string()],
            is_impossible: false,
        };
        let paragraph = ConvertedParagraph {
            id: "p1".to_string(),
            title: "title".to_string(),
            context: "context".to_string(),
            questions: vec![question],
        };

        ConvertedDataset {
            generated_at: Utc::now(),
            metadata: crate::datasets::DatasetMetadata::for_kind(
                DatasetKind::default(),
                false,
                None,
            ),
            source: "src".to_string(),
            paragraphs: vec![paragraph],
        }
    }

    fn dummy_slice<'a>(dataset: &'a ConvertedDataset) -> ResolvedSlice<'a> {
        let paragraph = &dataset.paragraphs[0];
        let question = &paragraph.questions[0];
        let manifest = SliceManifest {
            version: 1,
            slice_id: "slice-1".to_string(),
            dataset_id: dataset.metadata.id.clone(),
            dataset_label: dataset.metadata.label.clone(),
            dataset_source: dataset.source.clone(),
            includes_unanswerable: false,
            require_verified_chunks: false,
            seed: 1,
            requested_limit: Some(1),
            requested_corpus: 1,
            generated_at: Utc::now(),
            case_count: 1,
            positive_paragraphs: 1,
            negative_paragraphs: 0,
            total_paragraphs: 1,
            negative_multiplier: 1.0,
            cases: vec![SliceCaseEntry {
                question_id: question.id.clone(),
                paragraph_id: paragraph.id.clone(),
            }],
            paragraphs: vec![SliceParagraphEntry {
                id: paragraph.id.clone(),
                kind: SliceParagraphKind::Positive {
                    question_ids: vec![question.id.clone()],
                },
                shard_path: None,
            }],
        };

        ResolvedSlice {
            manifest,
            path: PathBuf::from("cache"),
            paragraphs: dataset.paragraphs.iter().collect(),
            cases: vec![CaseRef {
                paragraph,
                question,
            }],
        }
    }

    #[test]
    fn fingerprint_changes_with_chunk_settings() {
        let dataset = dummy_dataset();
        let slice = dummy_slice(&dataset);
        let checksum = "deadbeef";

        let base_config = IngestionConfig::default();
        let fp_base = build_ingestion_fingerprint(&dataset, &slice, checksum, &base_config);

        let mut token_config = base_config.clone();
        token_config.tuning.chunk_min_tokens += 1;
        let fp_token = build_ingestion_fingerprint(&dataset, &slice, checksum, &token_config);
        assert_ne!(fp_base, fp_token, "token bounds should affect fingerprint");

        let mut chunk_only_config = base_config;
        chunk_only_config.chunk_only = true;
        let fp_chunk_only =
            build_ingestion_fingerprint(&dataset, &slice, checksum, &chunk_only_config);
        assert_ne!(
            fp_base, fp_chunk_only,
            "chunk-only mode should affect fingerprint"
        );
    }
}
