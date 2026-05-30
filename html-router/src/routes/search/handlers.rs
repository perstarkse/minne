use std::collections::HashSet;

use axum::{
    extract::{Query, State},
};
use common::storage::types::{text_content::TextContent, user::User};
use retrieval_pipeline::{RetrievalConfig, SearchResult, SearchTarget, StrategyOutput};
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

pub async fn search_result_handler(
    State(state): State<HtmlState>,
    Query(params): Query<SearchParams>,
    RequireUser(user): RequireUser,
) -> TemplateResult {
    let (search_results_for_template, final_query_param_for_template) = if let Some(actual_query) =
        params.query
    {
        perform_search(&state, &user, actual_query).await?
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

async fn perform_search(
    state: &HtmlState,
    user: &User,
    query: String,
) -> Result<(Vec<SearchResultForTemplate>, String), HtmlError> {
    const TOTAL_LIMIT: usize = 10;

    let trimmed_query = query.trim();
    if trimmed_query.is_empty() {
        return Ok((Vec::new(), String::new()));
    }

    let config = RetrievalConfig::for_search(SearchTarget::Both);

    let reranker_lease = match &state.reranker_pool {
        Some(pool) => pool.checkout().await,
        None => None,
    };

    let params = retrieval_pipeline::pipeline::StrategyParams {
        db_client: &state.db,
        openai_client: &state.openai_client,
        embedding_provider: Some(&state.embedding_provider),
        input_text: trimmed_query,
        user_id: &user.id,
        config,
        reranker: reranker_lease,
    };
    let result = retrieval_pipeline::pipeline::execute(params).await?;

    let search_result = match result {
        StrategyOutput::Search(sr) => sr,
        _ => SearchResult::new(vec![], vec![]),
    };

    let source_label_map = collect_source_label_map(state, user, &search_result).await?;

    let mut combined_results: Vec<SearchResultForTemplate> =
        Vec::with_capacity(search_result.chunks.len().saturating_add(search_result.entities.len()));

    for chunk_result in search_result.chunks {
        let source_label = source_label_map
            .get(&chunk_result.chunk.source_id)
            .cloned()
            .unwrap_or_else(|| TextContent::fallback_source_label(&chunk_result.chunk.source_id));
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

    for entity_result in search_result.entities {
        let source_label = source_label_map
            .get(&entity_result.entity.source_id)
            .cloned()
            .unwrap_or_else(|| {
                TextContent::fallback_source_label(&entity_result.entity.source_id)
            });
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

    combined_results.sort_by(|a, b| b.score.total_cmp(&a.score));
    combined_results.truncate(TOTAL_LIMIT);

    Ok((combined_results, trimmed_query.to_string()))
}

async fn collect_source_label_map(
    state: &HtmlState,
    user: &User,
    search_result: &SearchResult,
) -> Result<std::collections::HashMap<String, String>, HtmlError> {
    let mut source_ids = HashSet::new();
    for chunk_result in &search_result.chunks {
        source_ids.insert(chunk_result.chunk.source_id.clone());
    }
    for entity_result in &search_result.entities {
        source_ids.insert(entity_result.entity.source_id.clone());
    }

    Ok(TextContent::resolve_source_labels(&state.db, &user.id, source_ids).await?)
}
