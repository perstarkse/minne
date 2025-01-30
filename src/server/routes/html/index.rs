use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect},
};
use axum_session::Session;
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::{engine::any::Any, Surreal};
use tracing::info;

use crate::{
    error::{AppError, HtmlError},
    page_data,
    server::{
        routes::html::{render_block, render_template},
        AppState,
    },
    storage::{
        db::{delete_item, get_all_stored_items, get_item},
        types::{
            file_info::FileInfo, job::Job, knowledge_entity::KnowledgeEntity,
            knowledge_relationship::KnowledgeRelationship, text_chunk::TextChunk,
            text_content::TextContent, user::User,
        },
    },
};

page_data!(IndexData, "index/index.html", {
    gdpr_accepted: bool,
    user: Option<User>,
    latest_text_contents: Vec<TextContent>,
    active_jobs: Vec<Job>
});

pub async fn index_handler(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    session: Session<SessionSurrealPool<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    info!("Displaying index page");

    let gdpr_accepted = auth.current_user.is_some() | session.get("gdpr_accepted").unwrap_or(false);

    let active_jobs = match auth.current_user.is_some() {
        true => state
            .job_queue
            .get_unfinished_user_jobs(&auth.current_user.clone().unwrap().id)
            .await
            .map_err(|e| HtmlError::new(e, state.templates.clone()))?,
        false => vec![],
    };

    let latest_text_contents = match auth.current_user.clone().is_some() {
        true => User::get_latest_text_contents(
            auth.current_user.clone().unwrap().id.as_str(),
            &state.surreal_db_client,
        )
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?,
        false => vec![],
    };

    // let latest_knowledge_entities = match auth.current_user.is_some() {
    //     true => User::get_latest_knowledge_entities(
    //         auth.current_user.clone().unwrap().id.as_str(),
    //         &state.surreal_db_client,
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
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    let user = match &auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    // Get TextContent from db
    let text_content = match get_item::<TextContent>(&state.surreal_db_client, &id)
        .await
        .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?
    {
        Some(text_content) => text_content,
        None => {
            return Err(HtmlError::new(
                AppError::NotFound("No item found".to_string()),
                state.templates,
            ))
        }
    };

    // Validate that the user is the owner
    if text_content.user_id != user.id {
        return Err(HtmlError::new(
            AppError::Auth("You are not the owner of that content".to_string()),
            state.templates,
        ));
    }

    // If TextContent has file_info, delete it from db and file from disk.
    if text_content.file_info.is_some() {
        FileInfo::delete_by_id(
            &text_content.file_info.unwrap().id,
            &state.surreal_db_client,
        )
        .await
        .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;
    }

    // Delete textcontent from db
    delete_item::<TextContent>(&state.surreal_db_client, &text_content.id)
        .await
        .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;

    // Delete TextChunks
    TextChunk::delete_by_source_id(&text_content.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    // Delete KnowledgeEntities
    KnowledgeEntity::delete_by_source_id(&text_content.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    // Delete KnowledgeRelationships
    KnowledgeRelationship::delete_relationships_by_source_id(
        &text_content.id,
        &state.surreal_db_client,
    )
    .await
    .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    // Get latest text contents after updates
    let latest_text_contents = User::get_latest_text_contents(&user.id, &state.surreal_db_client)
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

#[derive(Serialize)]
pub struct ActiveJobsData {
    active_jobs: Vec<Job>,
    user: User,
}

pub async fn delete_job(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/signin").into_response()),
    };

    state
        .job_queue
        .delete_job(&id, &user.id)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let active_jobs = state
        .job_queue
        .get_unfinished_user_jobs(&user.id)
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
