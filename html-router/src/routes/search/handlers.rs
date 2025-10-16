use std::{fmt, str::FromStr};

use axum::{
    extract::{Query, State},
    response::IntoResponse,
};
use common::storage::types::{
    conversation::Conversation,
    knowledge_entity::{KnowledgeEntity, KnowledgeEntitySearchResult},
    text_content::{TextContent, TextContentSearchResult},
    user::User,
};
use futures::future::try_join;
use serde::{de, Deserialize, Deserializer, Serialize};

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
        text_content: Option<TextContentSearchResult>,
        #[serde(skip_serializing_if = "Option::is_none")]
        knowledge_entity: Option<KnowledgeEntitySearchResult>,
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
                const TOTAL_LIMIT: usize = 10;
                let (text_results, entity_results) = try_join(
                    TextContent::search(&state.db, trimmed_query, &user.id, TOTAL_LIMIT),
                    KnowledgeEntity::search(&state.db, trimmed_query, &user.id, TOTAL_LIMIT),
                )
                .await?;

                let mut combined_results: Vec<SearchResultForTemplate> =
                    Vec::with_capacity(text_results.len() + entity_results.len());

                for text_result in text_results {
                    let score = text_result.score;
                    combined_results.push(SearchResultForTemplate {
                        result_type: "text_content".to_string(),
                        score,
                        text_content: Some(text_result),
                        knowledge_entity: None,
                    });
                }

                for entity_result in entity_results {
                    let score = entity_result.score;
                    combined_results.push(SearchResultForTemplate {
                        result_type: "knowledge_entity".to_string(),
                        score,
                        text_content: None,
                        knowledge_entity: Some(entity_result),
                    });
                }

                combined_results.sort_by(|a, b| b.score.total_cmp(&a.score));
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
