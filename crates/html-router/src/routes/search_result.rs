use axum::{
    extract::{Query, State},
    response::{IntoResponse, Redirect},
};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use serde::{Deserialize, Serialize};
use surrealdb::{engine::any::Any, Surreal};
use tracing::info;

use common::{error::HtmlError, storage::types::user::User};

use crate::{html_state::HtmlState, routes::render_template};
#[derive(Deserialize)]
pub struct SearchParams {
    query: String,
}

#[derive(Serialize)]
pub struct AnswerData {
    user_query: String,
    answer_content: String,
    answer_references: Vec<String>,
}

pub async fn search_result_handler(
    State(state): State<HtmlState>,
    Query(query): Query<SearchParams>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    info!("Displaying search results");

    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/signin").into_response()),
    };

    // let answer = get_answer_with_references(
    //     &state.surreal_db_client,
    //     &state.openai_client,
    //     &query.query,
    //     &user.id,
    // )
    // .await
    // .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let answer = "The Minne project is focused on simplifying knowledge management through features such as easy capture, smart analysis, and visualization of connections between ideas. It includes various functionalities like the Smart Analysis Feature, which provides content analysis and organization, and the Easy Capture Feature, which allows users to effortlessly capture and retrieve knowledge in various formats. Additionally, it offers tools like Knowledge Graph Visualization to enhance understanding and organization of knowledge. The project also emphasizes a user-friendly onboarding experience and mobile-friendly options for accessing its services.".to_string();

    let references = vec![
        "i81cd5be8-557c-4b2b-ba3a-4b8d28e74b9b".to_string(),
        "5f72a724-d7a3-467d-8783-7cca6053ddc7".to_string(),
        "ad106a1f-ccda-415e-9e87-c3a34e202624".to_string(),
        "8797b57d-094d-4ee9-a3a7-c3195b246254".to_string(),
        "69763f43-82e6-4cb5-ba3e-f6da13777dab".to_string(),
    ];

    let output = render_template(
        "index/signed_in/search_response.html",
        AnswerData {
            user_query: query.query,
            answer_content: answer,
            answer_references: references,
        },
        state.templates,
    )?;

    Ok(output.into_response())
}
