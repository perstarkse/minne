use std::{pin::Pin, time::Duration};

use axum::{
    extract::{Query, State},
    response::{
        sse::{Event, KeepAlive},
        Html, IntoResponse, Sse,
    },
};
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use futures::{future::try_join_all, stream, Stream, StreamExt, TryFutureExt};
use minijinja::context;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use tokio::time::sleep;
use tracing::{error, info};

use common::{
    error::AppError,
    storage::types::{
        file_info::FileInfo,
        ingestion_payload::IngestionPayload,
        ingestion_task::{IngestionTask, TaskState},
        user::User,
    },
};

use crate::{
    html_state::HtmlState,
    middlewares::{
        auth_middleware::RequireUser,
        response_middleware::{HtmlError, TemplateResponse},
    },
    AuthSessionType,
};

pub async fn show_ingress_form(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    let user_categories = User::get_user_categories(&user.id, &state.db).await?;

    #[derive(Serialize)]
    pub struct ShowIngressFormData {
        user_categories: Vec<String>,
    }

    Ok(TemplateResponse::new_template(
        "ingestion_modal.html",
        ShowIngressFormData { user_categories },
    ))
}

pub async fn hide_ingress_form(
    RequireUser(_user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    Ok(Html(
        "<a class='btn btn-primary' hx-get='/ingress-form' hx-swap='outerHTML'>Add Content</a>",
    )
    .into_response())
}

#[derive(Debug, TryFromMultipart)]
pub struct IngestionParams {
    pub content: Option<String>,
    pub context: String,
    pub category: String,
    #[form_data(limit = "10000000")] // Adjust limit as needed
    #[form_data(default)]
    pub files: Vec<FieldData<NamedTempFile>>,
}

pub async fn process_ingress_form(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    TypedMultipart(input): TypedMultipart<IngestionParams>,
) -> Result<impl IntoResponse, HtmlError> {
    #[derive(Serialize)]
    pub struct IngressFormData {
        context: String,
        content: String,
        category: String,
        error: String,
    }

    if input.content.as_ref().is_none_or(|c| c.len() < 2) && input.files.is_empty() {
        return Ok(TemplateResponse::new_template(
            "index/signed_in/ingress_form.html",
            IngressFormData {
                context: input.context.clone(),
                content: input.content.clone().unwrap_or_default(),
                category: input.category.clone(),
                error: "You need to either add files or content".to_string(),
            },
        ));
    }

    let content_bytes = input.content.as_ref().map_or(0, |c| c.len());
    let has_content = input.content.as_ref().is_some_and(|c| !c.trim().is_empty());
    let context_bytes = input.context.len();
    let category_bytes = input.category.len();
    let file_count = input.files.len();

    info!(
        user_id = %user.id,
        has_content,
        content_bytes,
        context_bytes,
        category_bytes,
        file_count,
        "Received ingestion form submission"
    );

    let file_infos = try_join_all(input.files.into_iter().map(|file| {
        FileInfo::new_with_storage(file, &state.db, &user.id, &state.storage)
            .map_err(AppError::from)
    }))
    .await?;

    let payloads = IngestionPayload::create_ingestion_payload(
        input.content,
        input.context,
        input.category,
        file_infos,
        user.id.as_str(),
    )?;

    let futures: Vec<_> = payloads
        .into_iter()
        .map(|object| IngestionTask::create_and_add_to_db(object, user.id.clone(), &state.db))
        .collect();

    let tasks = try_join_all(futures).await?;

    #[derive(Serialize)]
    struct NewTasksData {
        tasks: Vec<IngestionTask>,
    }

    Ok(TemplateResponse::new_template(
        "dashboard/current_task.html",
        NewTasksData { tasks },
    ))
}

#[derive(Deserialize)]
pub struct QueryParams {
    task_id: String,
}

fn create_error_stream(
    message: impl Into<String>,
) -> Pin<Box<dyn Stream<Item = Result<Event, axum::Error>> + Send>> {
    let message = message.into();
    stream::once(async move { Ok(Event::default().event("error").data(message)) }).boxed()
}

pub async fn get_task_updates_stream(
    State(state): State<HtmlState>,
    auth: AuthSessionType,
    Query(params): Query<QueryParams>,
) -> Sse<Pin<Box<dyn Stream<Item = Result<Event, axum::Error>> + Send>>> {
    let task_id = params.task_id.clone();
    let db = state.db.clone();

    // 1. Check for authenticated user
    let Some(current_user) = auth.current_user else {
        return Sse::new(create_error_stream("User not authenticated"));
    };

    // 2. Fetch task for initial authorization and to ensure it exists
    match db.get_item::<IngestionTask>(&task_id).await {
        Ok(Some(task)) => {
            // 3. Validate user ownership
            if task.user_id != current_user.id {
                return Sse::new(create_error_stream(
                    "Access denied: You do not have permission to view updates for this task.",
                ));
            }

            let sse_stream = async_stream::stream! {
                let mut consecutive_db_errors = 0;
                let max_consecutive_db_errors = 3;

                loop {
                    match db.get_item::<IngestionTask>(&task_id).await {
                        Ok(Some(updated_task)) => {
                            consecutive_db_errors = 0; // Reset error count on success

                            let status_message = match updated_task.state {
                                TaskState::Pending => "Pending".to_string(),
                                TaskState::Reserved => format!(
                                    "Reserved (attempt {} of {})",
                                    updated_task.attempts,
                                    updated_task.max_attempts
                                ),
                                TaskState::Processing => format!(
                                    "Processing (attempt {} of {})",
                                    updated_task.attempts,
                                    updated_task.max_attempts
                                ),
                                TaskState::Succeeded => "Completed".to_string(),
                                TaskState::Failed => {
                                    let mut base = format!(
                                        "Retry scheduled (attempt {} of {})",
                                        updated_task.attempts,
                                        updated_task.max_attempts
                                    );
                                    if let Some(message) = updated_task.error_message.as_ref() {
                                        base.push_str(": ");
                                        base.push_str(message);
                                    }
                                    base
                                }
                                TaskState::Cancelled => "Cancelled".to_string(),
                                TaskState::DeadLetter => {
                                    let mut base = "Failed permanently".to_string();
                                    if let Some(message) = updated_task.error_message.as_ref() {
                                        base.push_str(": ");
                                        base.push_str(message);
                                    }
                                    base
                                }
                            };

                            yield Ok(Event::default().event("status").data(status_message));

                            // Check for terminal states to close the stream
                            if updated_task.state.is_terminal() {
                                // Send a specific event that HTMX uses to close the connection
                                // Send a event to reload the recent content
                                // Send a event to remove the loading indicatior
                                let check_icon = state.templates.render("icons/check_icon.html", &context!{}).unwrap_or_else(|_| "Ok".to_string());
                                yield Ok(Event::default().event("stop_loading").data(check_icon));
                                yield Ok(Event::default().event("update_latest_content").data("Update latest content"));
                                yield Ok(Event::default().event("close_stream").data("Stream complete"));
                                break; // Exit loop on terminal states
                            }
                        },
                        Ok(None) => {
                            // Task disappeared after initial fetch
                            yield Ok(Event::default().event("error").data("Task not found during update polling."));
                            break;
                        }
                        Err(db_err) => {
                            error!("Database error while fetching task '{}': {:?}", task_id, db_err);
                            consecutive_db_errors += 1;
                            yield Ok(Event::default().event("error").data(format!("Temporary error fetching task update (attempt {consecutive_db_errors}).")));

                            if consecutive_db_errors >= max_consecutive_db_errors {
                                error!("Max consecutive DB errors reached for task '{task_id}'. Closing stream.");
                                yield Ok(Event::default().event("error").data("Persistent error fetching task updates. Stream closed."));
                                yield Ok(Event::default().event("close_stream").data("Stream complete"));
                                break;
                            }
                        }
                    }
                    sleep(Duration::from_secs(2)).await;
                }
            };

            Sse::new(sse_stream.boxed()).keep_alive(
                KeepAlive::new()
                    .interval(Duration::from_secs(15))
                    .text("keep-alive-ping"),
            )
        }
        Ok(None) => Sse::new(create_error_stream(format!(
            "Task with ID '{task_id}' not found."
        ))),
        Err(e) => {
            error!("Failed to fetch task '{task_id}' for authorization: {e:?}");
            Sse::new(create_error_stream(
                "An error occurred while retrieving task details. Please try again later.",
            ))
        }
    }
}
