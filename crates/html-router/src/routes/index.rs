use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect},
};
use axum_session::Session;
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::{engine::any::Any, Surreal};
use tokio::join;
use tracing::info;

use common::{
    error::{AppError, HtmlError},
    storage::types::{
        file_info::FileInfo, ingestion_task::IngestionTask, knowledge_entity::KnowledgeEntity,
        knowledge_relationship::KnowledgeRelationship, text_chunk::TextChunk,
        text_content::TextContent, user::User,
    },
};

use crate::{html_state::HtmlState, page_data, routes::render_template};

use super::render_block;

page_data!(IndexData, "index/index.html", {
    gdpr_accepted: bool,
    user: Option<User>,
    latest_text_contents: Vec<TextContent>,
    active_jobs: Vec<IngestionTask>
});

pub async fn index_handler(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    session: Session<SessionSurrealPool<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    info!("Displaying index page");

    let gdpr_accepted = auth.current_user.is_some() | session.get("gdpr_accepted").unwrap_or(false);

    let active_jobs = match auth.current_user.is_some() {
        true => {
            User::get_unfinished_ingestion_tasks(&auth.current_user.clone().unwrap().id, &state.db)
                .await
                .map_err(|e| HtmlError::new(e, state.templates.clone()))?
        }
        false => vec![],
    };

    let latest_text_contents = match auth.current_user.clone().is_some() {
        true => User::get_latest_text_contents(
            auth.current_user.clone().unwrap().id.as_str(),
            &state.db,
        )
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?,
        false => vec![],
    };

    // let latest_knowledge_entities = match auth.current_user.is_some() {
    //     true => User::get_latest_knowledge_entities(
    //         auth.current_user.clone().unwrap().id.as_str(),
    //         &state.db,
    //     )
    //     .await
    //     .map_err(|e| HtmlError::new(e, state.templates.clone()))?,
    //     false => vec![],
    // };

    let output = render_template(
        IndexData::template_name(),
        IndexData {
            gdpr_accepted,
            user: auth.current_user,
            latest_text_contents,
            active_jobs,
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}

#[derive(Serialize)]
pub struct LatestTextContentData {
    latest_text_contents: Vec<TextContent>,
    user: User,
}

pub async fn delete_text_content(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    let user = match &auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    // Get and validate TextContent
    let text_content = get_and_validate_text_content(&state, &id, user).await?;

    // Perform concurrent deletions
    let deletion_tasks = join!(
        async {
            if let Some(file_info) = text_content.file_info {
                FileInfo::delete_by_id(&file_info.id, &state.db).await
            } else {
                Ok(())
            }
        },
        state.db.delete_item::<TextContent>(&text_content.id),
        TextChunk::delete_by_source_id(&text_content.id, &state.db),
        KnowledgeEntity::delete_by_source_id(&text_content.id, &state.db),
        KnowledgeRelationship::delete_relationships_by_source_id(&text_content.id, &state.db)
    );

    // Handle potential errors from concurrent operations
    match deletion_tasks {
        (Ok(_), Ok(_), Ok(_), Ok(_), Ok(_)) => (),
        _ => {
            return Err(HtmlError::new(
                AppError::Processing("Failed to delete one or more items".to_string()),
                state.templates.clone(),
            ))
        }
    }

    // Render updated content
    let latest_text_contents = User::get_latest_text_contents(&user.id, &state.db)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let output = render_block(
        "index/signed_in/recent_content.html",
        "latest_content_section",
        LatestTextContentData {
            user: user.clone(),
            latest_text_contents,
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}

// Helper function to get and validate text content
async fn get_and_validate_text_content(
    state: &HtmlState,
    id: &str,
    user: &User,
) -> Result<TextContent, HtmlError> {
    let text_content = state
        .db
        .get_item::<TextContent>(id)
        .await
        .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?
        .ok_or_else(|| {
            HtmlError::new(
                AppError::NotFound("No item found".to_string()),
                state.templates.clone(),
            )
        })?;

    if text_content.user_id != user.id {
        return Err(HtmlError::new(
            AppError::Auth("You are not the owner of that content".to_string()),
            state.templates.clone(),
        ));
    }

    Ok(text_content)
}

#[derive(Serialize)]
pub struct ActiveJobsData {
    pub active_jobs: Vec<IngestionTask>,
    pub user: User,
}

pub async fn delete_job(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/signin").into_response()),
    };

    User::validate_and_delete_job(&id, &user.id, &state.db)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let active_jobs = User::get_unfinished_ingestion_tasks(&user.id, &state.db)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let output = render_block(
        "index/signed_in/active_jobs.html",
        "active_jobs_section",
        ActiveJobsData {
            user: user.clone(),
            active_jobs,
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}

pub async fn show_active_jobs(
    State(state): State<HtmlState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/signin").into_response()),
    };

    let active_jobs = User::get_unfinished_ingestion_tasks(&user.id, &state.db)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let output = render_block(
        "index/signed_in/active_jobs.html",
        "active_jobs_section",
        ActiveJobsData {
            user: user.clone(),
            active_jobs,
        },
        state.templates.clone(),
    )?;

    Ok(output.into_response())
}
