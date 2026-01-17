use std::{
    collections::{HashMap, HashSet},
    fmt,
    str::FromStr,
};

use axum::{
    extract::{Query, State},
    response::IntoResponse,
};
use common::storage::types::{
    text_content::{deserialize_flexible_id, TextContent},
    StoredObject,
};
use retrieval_pipeline::{RetrievalConfig, SearchResult, SearchTarget, StrategyOutput};
use serde::{de, Deserialize, Deserializer, Serialize};
use surrealdb::RecordId;

use crate::{
    html_state::HtmlState,
    middlewares::{
        auth_middleware::RequireUser,
        response_middleware::{HtmlError, TemplateResponse},
    },
};

/// Serde deserialization decorator to map empty Strings to None,
fn empty_string_as_none<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: fmt::Display,
{
    let opt = Option::<String>::deserialize(de)?;
    match opt.as_deref() {
        None | Some("") => Ok(None),
        Some(s) => FromStr::from_str(s).map_err(de::Error::custom).map(Some),
    }
}

fn source_id_suffix(source_id: &str) -> String {
    let start = source_id.len().saturating_sub(8);
    source_id[start..].to_string()
}

fn truncate_label(value: &str, max_chars: usize) -> String {
    let mut end = None;
    let mut count = 0;
    for (idx, _) in value.char_indices() {
        if count == max_chars {
            end = Some(idx);
            break;
        }
        count += 1;
    }

    match end {
        Some(idx) => format!("{}...", &value[..idx]),
        None => value.to_string(),
    }
}

fn first_non_empty_line(text: &str, max_chars: usize) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            return Some(truncate_label(trimmed, max_chars));
        }
    }
    None
}

#[derive(Deserialize)]
struct UrlInfoLabel {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
}

#[derive(Deserialize)]
struct FileInfoLabel {
    #[serde(default)]
    file_name: String,
}

#[derive(Deserialize)]
struct SourceLabelRow {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    id: String,
    #[serde(default)]
    url_info: Option<UrlInfoLabel>,
    #[serde(default)]
    file_info: Option<FileInfoLabel>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    category: String,
    #[serde(default)]
    text: String,
}

fn build_source_label(row: &SourceLabelRow) -> String {
    const MAX_LABEL_CHARS: usize = 80;

    if let Some(url_info) = row.url_info.as_ref() {
        let title = url_info.title.trim();
        if !title.is_empty() {
            return title.to_string();
        }

        let url = url_info.url.trim();
        if !url.is_empty() {
            return url.to_string();
        }
    }

    if let Some(file_info) = row.file_info.as_ref() {
        let name = file_info.file_name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }

    if let Some(context) = row.context.as_ref() {
        let trimmed = context.trim();
        if !trimmed.is_empty() {
            return truncate_label(trimmed, MAX_LABEL_CHARS);
        }
    }

    if let Some(text_label) = first_non_empty_line(&row.text, MAX_LABEL_CHARS) {
        return text_label;
    }

    let category = row.category.trim();
    if !category.is_empty() {
        return truncate_label(category, MAX_LABEL_CHARS);
    }

    format!("Text snippet: {}", source_id_suffix(&row.id))
}

fn fallback_source_label(source_id: &str) -> String {
    format!("Text snippet: {}", source_id_suffix(source_id))
}

#[derive(Deserialize)]
pub struct SearchParams {
    #[serde(default, deserialize_with = "empty_string_as_none")]
    query: Option<String>,
}

/// Chunk result for template rendering
#[derive(Serialize)]
struct TextChunkForTemplate {
    id: String,
    source_id: String,
    source_label: String,
    chunk: String,
    score: f32,
}

/// Entity result for template rendering (from pipeline)
#[derive(Serialize)]
struct KnowledgeEntityForTemplate {
    id: String,
    name: String,
    description: String,
    entity_type: String,
    source_id: String,
    source_label: String,
    score: f32,
}

pub async fn search_result_handler(
    State(state): State<HtmlState>,
    Query(params): Query<SearchParams>,
    RequireUser(user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    #[derive(Serialize)]
    struct SearchResultForTemplate {
        result_type: String,
        score: f32,
        #[serde(skip_serializing_if = "Option::is_none")]
        text_chunk: Option<TextChunkForTemplate>,
        #[serde(skip_serializing_if = "Option::is_none")]
        knowledge_entity: Option<KnowledgeEntityForTemplate>,
    }

    #[derive(Serialize)]
    pub struct AnswerData {
        search_result: Vec<SearchResultForTemplate>,
        query_param: String,
    }

    let (search_results_for_template, final_query_param_for_template) = if let Some(actual_query) =
        params.query
    {
        let trimmed_query = actual_query.trim();
        if trimmed_query.is_empty() {
            (Vec::<SearchResultForTemplate>::new(), String::new())
        } else {
            // Use retrieval pipeline Search strategy
            let config = RetrievalConfig::for_search(SearchTarget::Both);

            // Checkout a reranker lease if pool is available
            let reranker_lease = match &state.reranker_pool {
                Some(pool) => Some(pool.checkout().await),
                None => None,
            };

            let result = retrieval_pipeline::pipeline::run_pipeline(
                &state.db,
                &state.openai_client,
                Some(&state.embedding_provider),
                trimmed_query,
                &user.id,
                config,
                reranker_lease,
            )
            .await?;

            let search_result = match result {
                StrategyOutput::Search(sr) => sr,
                _ => SearchResult::new(vec![], vec![]),
            };

            let mut source_ids = HashSet::new();
            for chunk_result in &search_result.chunks {
                source_ids.insert(chunk_result.chunk.source_id.clone());
            }
            for entity_result in &search_result.entities {
                source_ids.insert(entity_result.entity.source_id.clone());
            }

            let source_label_map = if source_ids.is_empty() {
                HashMap::new()
            } else {
                let record_ids: Vec<RecordId> = source_ids
                    .iter()
                    .filter_map(|id| {
                        if id.contains(':') {
                            RecordId::from_str(id).ok()
                        } else {
                            Some(RecordId::from_table_key(TextContent::table_name(), id))
                        }
                    })
                    .collect();
                let mut response = state
                        .db
                        .client
                        .query(
                            "SELECT id, url_info, file_info, context, category, text FROM type::table($table_name) WHERE user_id = $user_id AND id INSIDE $record_ids",
                        )
                        .bind(("table_name", TextContent::table_name()))
                        .bind(("user_id", user.id.clone()))
                        .bind(("record_ids", record_ids))
                        .await?;
                let contents: Vec<SourceLabelRow> = response.take(0)?;

                tracing::debug!(
                    source_id_count = source_ids.len(),
                    label_row_count = contents.len(),
                    "Resolved search source labels"
                );

                let mut labels = HashMap::new();
                for content in contents {
                    let label = build_source_label(&content);
                    labels.insert(content.id.clone(), label.clone());
                    labels.insert(
                        format!("{}:{}", TextContent::table_name(), content.id),
                        label,
                    );
                }

                labels
            };

            let mut combined_results: Vec<SearchResultForTemplate> =
                Vec::with_capacity(search_result.chunks.len() + search_result.entities.len());

            // Add chunk results
            for chunk_result in search_result.chunks {
                let source_label = source_label_map
                    .get(&chunk_result.chunk.source_id)
                    .cloned()
                    .unwrap_or_else(|| fallback_source_label(&chunk_result.chunk.source_id));
                combined_results.push(SearchResultForTemplate {
                    result_type: "text_chunk".to_string(),
                    score: chunk_result.score,
                    text_chunk: Some(TextChunkForTemplate {
                        id: chunk_result.chunk.id,
                        source_id: chunk_result.chunk.source_id,
                        source_label,
                        chunk: chunk_result.chunk.chunk,
                        score: chunk_result.score,
                    }),
                    knowledge_entity: None,
                });
            }

            // Add entity results
            for entity_result in search_result.entities {
                let source_label = source_label_map
                    .get(&entity_result.entity.source_id)
                    .cloned()
                    .unwrap_or_else(|| fallback_source_label(&entity_result.entity.source_id));
                combined_results.push(SearchResultForTemplate {
                    result_type: "knowledge_entity".to_string(),
                    score: entity_result.score,
                    text_chunk: None,
                    knowledge_entity: Some(KnowledgeEntityForTemplate {
                        id: entity_result.entity.id,
                        name: entity_result.entity.name,
                        description: entity_result.entity.description,
                        entity_type: format!("{:?}", entity_result.entity.entity_type),
                        source_id: entity_result.entity.source_id,
                        source_label,
                        score: entity_result.score,
                    }),
                });
            }

            // Sort by score descending
            combined_results.sort_by(|a, b| b.score.total_cmp(&a.score));

            // Limit results
            const TOTAL_LIMIT: usize = 10;
            combined_results.truncate(TOTAL_LIMIT);

            (combined_results, trimmed_query.to_string())
        }
    } else {
        (Vec::<SearchResultForTemplate>::new(), String::new())
    };

    Ok(TemplateResponse::new_template(
        "search/base.html",
        AnswerData {
            search_result: search_results_for_template,
            query_param: final_query_param_for_template,
        },
    ))
}
