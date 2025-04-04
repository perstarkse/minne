use axum::{
    extract::{Query, State},
    response::IntoResponse,
};
use composite_retrieval::answer_retrieval::get_answer_with_references;
use serde::{Deserialize, Serialize};

use crate::{
    html_state::HtmlState,
    middlewares::{
        auth_middleware::RequireUser,
        response_middleware::{HtmlError, TemplateResponse},
    },
};

#[derive(Deserialize)]
pub struct SearchParams {
    query: String,
}

pub async fn search_result_handler(
    State(state): State<HtmlState>,
    Query(query): Query<SearchParams>,
    RequireUser(user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    #[derive(Serialize)]
    pub struct AnswerData {
        user_query: String,
        answer_content: String,
        answer_references: Vec<String>,
    }

    let answer =
        get_answer_with_references(&state.db, &state.openai_client, &query.query, &user.id).await?;

    Ok(TemplateResponse::new_template(
        "index/signed_in/search_response.html",
        AnswerData {
            user_query: query.query,
            answer_content: answer.content,
            answer_references: answer.references,
        },
    ))
}
