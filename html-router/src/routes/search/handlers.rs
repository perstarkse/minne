use std::collections::HashSet;

use axum::{
    extract::{Query, State},
};
use axum_htmx::{HxBoosted, HxRequest};
use common::storage::types::{text_content::TextContent, user::User};
use retrieval_pipeline::{retrieve, RetrievalConfig, RetrievalOutput, RetrievedChunk, RetrievedEntity};
use serde::{de, Deserialize, Deserializer, Serialize};
use std::{fmt, str::FromStr};

use crate::{
    html_state::HtmlState,
    middlewares::{
        auth_middleware::RequireUser,
        response_middleware::{HtmlError, TemplateResponse, TemplateResult},
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
enum SearchView {
    #[default]
    All,
    Chunks,
    Entities,
}

impl<'de> Deserialize<'de> for SearchView {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let opt = Option::<String>::deserialize(deserializer)?;
        Ok(match opt.as_deref() {
            None | Some("" | "all") => SearchView::All,
            Some("chunks") => SearchView::Chunks,
            Some("entities") => SearchView::Entities,
            Some(other) => {
                return Err(de::Error::custom(format!(
                    "invalid search view: {other}"
                )));
            }
        })
    }
}

impl SearchView {
    fn as_str(self) -> &'static str {
        match self {
            SearchView::All => "all",
            SearchView::Chunks => "chunks",
            SearchView::Entities => "entities",
        }
    }
}

#[derive(Deserialize)]
pub struct SearchParams {
    #[serde(default, deserialize_with = "empty_string_as_none")]
    query: Option<String>,
    #[serde(default)]
    view: SearchView,
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
    view_param: String,
}

pub async fn search_result_handler(
    State(state): State<HtmlState>,
    Query(params): Query<SearchParams>,
    RequireUser(user): RequireUser,
    HxRequest(is_htmx): HxRequest,
    HxBoosted(is_boosted): HxBoosted,
) -> TemplateResult {
    let view = params.view;
    let (search_results_for_template, final_query_param_for_template) = if let Some(actual_query) =
        params.query
    {
        perform_search(&state, &user, actual_query, view).await?
    } else {
        (Vec::<SearchResultForTemplate>::new(), String::new())
    };

    let data = AnswerData {
        search_result: search_results_for_template,
        query_param: final_query_param_for_template,
        view_param: view.as_str().to_string(),
    };

    if is_htmx && !is_boosted {
        Ok(TemplateResponse::new_partial(
            "search/base.html",
            "main",
            data,
        ))
    } else {
        Ok(TemplateResponse::new_template("search/base.html", data))
    }
}

async fn perform_search(
    state: &HtmlState,
    user: &User,
    query: String,
    view: SearchView,
) -> Result<(Vec<SearchResultForTemplate>, String), HtmlError> {
    const TOTAL_LIMIT: usize = 10;

    let trimmed_query = query.trim();
    if trimmed_query.is_empty() {
        return Ok((Vec::new(), String::new()));
    }

    let config = match view {
        SearchView::Chunks => RetrievalConfig::default(),
        SearchView::All | SearchView::Entities => RetrievalConfig::with_entities(),
    };

    let reranker_lease = match &state.reranker_pool {
        Some(pool) => pool.checkout().await,
        None => None,
    };

    let result = retrieve(
        &state.db,
        &state.embedding_provider,
        trimmed_query,
        &user.id,
        config,
        reranker_lease,
    )
    .await?;

    let mut results = match view {
        SearchView::Chunks => {
            let chunks = match result {
                RetrievalOutput::Chunks(chunks) | RetrievalOutput::WithEntities { chunks, .. } => {
                    chunks
                }
            };
            let source_label_map = collect_source_label_map(state, user, &chunks, &[]).await?;
            chunk_results_for_template(&chunks, &source_label_map)
        }
        SearchView::Entities => {
            let entities = match result {
                RetrievalOutput::WithEntities { entities, .. } => entities,
                RetrievalOutput::Chunks(_) => Vec::new(),
            };
            let source_label_map = collect_source_label_map(state, user, &[], &entities).await?;
            entity_results_for_template(&entities, &source_label_map)
        }
        SearchView::All => {
            let (chunks, entities) = match result {
                RetrievalOutput::WithEntities { chunks, entities } => (chunks, entities),
                RetrievalOutput::Chunks(chunks) => (chunks, Vec::new()),
            };
            let source_label_map =
                collect_source_label_map(state, user, &chunks, &entities).await?;
            let mut combined = chunk_results_for_template(&chunks, &source_label_map);
            combined.extend(entity_results_for_template(&entities, &source_label_map));
            combined
        }
    };

    results.sort_by(|a, b| b.score.total_cmp(&a.score));
    results.truncate(TOTAL_LIMIT);

    Ok((results, trimmed_query.to_string()))
}

fn chunk_results_for_template(
    chunks: &[RetrievedChunk],
    source_label_map: &std::collections::HashMap<String, String>,
) -> Vec<SearchResultForTemplate> {
    chunks
        .iter()
        .map(|chunk_result| {
            let source_label = source_label_map
                .get(&chunk_result.chunk.source_id)
                .cloned()
                .unwrap_or_else(|| {
                    TextContent::fallback_source_label(&chunk_result.chunk.source_id)
                });
            SearchResultForTemplate {
                result_type: "text_chunk".to_string(),
                score: chunk_result.score,
                text_chunk: Some(TextChunkForTemplate {
                    id: chunk_result.chunk.id.clone(),
                    source_id: chunk_result.chunk.source_id.clone(),
                    source_label,
                    chunk: chunk_result.chunk.chunk.clone(),
                    score: chunk_result.score,
                }),
                knowledge_entity: None,
            }
        })
        .collect()
}

fn entity_results_for_template(
    entities: &[RetrievedEntity],
    source_label_map: &std::collections::HashMap<String, String>,
) -> Vec<SearchResultForTemplate> {
    entities
        .iter()
        .map(|entity_result| {
            let source_label = source_label_map
                .get(&entity_result.entity.source_id)
                .cloned()
                .unwrap_or_else(|| {
                    TextContent::fallback_source_label(&entity_result.entity.source_id)
                });
            SearchResultForTemplate {
                result_type: "knowledge_entity".to_string(),
                score: entity_result.score,
                text_chunk: None,
                knowledge_entity: Some(KnowledgeEntityForTemplate {
                    id: entity_result.entity.id.clone(),
                    name: entity_result.entity.name.clone(),
                    description: entity_result.entity.description.clone(),
                    entity_type: format!("{:?}", entity_result.entity.entity_type),
                    source_id: entity_result.entity.source_id.clone(),
                    source_label,
                    score: entity_result.score,
                }),
            }
        })
        .collect()
}

async fn collect_source_label_map(
    state: &HtmlState,
    user: &User,
    chunks: &[RetrievedChunk],
    entities: &[RetrievedEntity],
) -> Result<std::collections::HashMap<String, String>, HtmlError> {
    let mut source_ids = HashSet::new();
    for chunk_result in chunks {
        source_ids.insert(chunk_result.chunk.source_id.clone());
    }
    for entity_result in entities {
        source_ids.insert(entity_result.entity.source_id.clone());
    }

    Ok(TextContent::resolve_source_labels(&state.db, &user.id, source_ids).await?)
}
