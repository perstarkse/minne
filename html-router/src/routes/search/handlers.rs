use std::{fmt, str::FromStr, time::Duration};

use axum::{
    extract::{Query, State},
    response::IntoResponse,
};
use common::storage::types::{conversation::Conversation, user::User};
use retrieval_pipeline::{RetrievalConfig, SearchResult, SearchTarget, StrategyOutput};
use serde::{de, Deserialize, Deserializer, Serialize};
use tokio::time::error::Elapsed;

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
        user: User,
        conversation_archive: Vec<Conversation>,
    }
    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    let (search_results_for_template, final_query_param_for_template) =
        if let Some(actual_query) = params.query {
            let trimmed_query = actual_query.trim();
            if trimmed_query.is_empty() {
                (Vec::<SearchResultForTemplate>::new(), String::new())
            } else {
                // Use retrieval pipeline Search strategy
                let config = RetrievalConfig::for_search(SearchTarget::Both);
                let result = retrieval_pipeline::pipeline::run_pipeline(
                    &state.db,
                    &state.openai_client,
                    None, // No embedding provider in HtmlState
                    trimmed_query,
                    &user.id,
                    config,
                    None, // No reranker for now
                )
                .await?;

                let search_result = match result {
                    StrategyOutput::Search(sr) => sr,
                    _ => SearchResult::new(vec![], vec![]),
                };

                let mut combined_results: Vec<SearchResultForTemplate> =
                    Vec::with_capacity(search_result.chunks.len() + search_result.entities.len());

                // Add chunk results
                for chunk_result in search_result.chunks {
                    combined_results.push(SearchResultForTemplate {
                        result_type: "text_chunk".to_string(),
                        score: chunk_result.score,
                        text_chunk: Some(TextChunkForTemplate {
                            id: chunk_result.chunk.id,
                            source_id: chunk_result.chunk.source_id,
                            chunk: chunk_result.chunk.chunk,
                            score: chunk_result.score,
                        }),
                        knowledge_entity: None,
                    });
                }

                // Add entity results
                for entity_result in search_result.entities {
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
            user,
            conversation_archive,
        },
    ))
}
