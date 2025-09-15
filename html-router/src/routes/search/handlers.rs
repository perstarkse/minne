use std::{fmt, str::FromStr};

use axum::{
    extract::{Query, State},
    response::IntoResponse,
};
use common::storage::types::{
    conversation::Conversation,
    text_content::{TextContent, TextContentSearchResult},
    user::User,
};
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
    pub struct AnswerData {
        search_result: Vec<TextContentSearchResult>,
        query_param: String,
        user: User,
        conversation_archive: Vec<Conversation>,
    }
    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    let (search_results_for_template, final_query_param_for_template) =
        if let Some(actual_query) = params.query {
            let trimmed_query = actual_query.trim();
            if trimmed_query.is_empty() {
                (Vec::new(), String::new())
            } else {
                match TextContent::search(&state.db, trimmed_query, &user.id, 5).await {
                    Ok(results) => (results, trimmed_query.to_string()),
                    Err(e) => {
                        return Err(HtmlError::from(e));
                    }
                }
            }
        } else {
            (Vec::new(), String::new())
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
