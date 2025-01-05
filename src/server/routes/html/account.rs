use std::collections::HashMap;

use axum::{
    extract::{Query, State},
    http::{StatusCode, Uri},
    response::{Html, IntoResponse, Redirect},
};
use axum_htmx::HxRedirect;
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use surrealdb::{engine::any::Any, Surreal};

use crate::{
    error::{AppError, HtmlError},
    page_data,
    server::{routes::html::render_template, AppState},
    storage::{db::delete_item, types::user::User},
};

use super::render_block;

page_data!(AccountData, "auth/account.html", {
    user: User
});

pub async fn show_account_page(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    let output = render_template(
        AccountData::template_name(),
        AccountData { user },
        state.templates.clone(),
    )
    .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;

    Ok(output.into_response())
}

pub async fn set_api_key(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match &auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    // Generate and set the API key
    let api_key = User::set_api_key(&user.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    auth.cache_clear_user(user.id.to_string());

    // Update the user's API key
    let updated_user = User {
        api_key: Some(api_key),
        ..user.clone()
    };

    // Render the API key section block
    let output = render_block(
        AccountData::template_name(),
        "api_key_section",
        AccountData { user: updated_user },
        state.templates.clone(),
    )
    .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;

    Ok(output.into_response())
}

pub async fn delete_account(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match &auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };

    delete_item::<User>(&state.surreal_db_client, &user.id)
        .await
        .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;

    auth.logout_user();

    auth.session.destroy();

    Ok((HxRedirect::from(Uri::from_static("/")), StatusCode::OK).into_response())
}

pub async fn show_ios_shortcut(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    let user = match &auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/").into_response()),
    };
    let base_url = "https://minne.starks.cloud";
    let shortcut_url = format!(
        "{}/api/shortcuts/template?key={:?}",
        base_url,
        user.api_key.as_ref().unwrap()
    );
    let deep_link = format!("shortcuts://import-workflow?url={}", &shortcut_url);

    Ok(Html(format!(
        r#"
         <div class="p-4 mt-4 flex flex-col justify-center items-center border rounded-lg bg-base-200">
             <h3 class="text-lg font-bold mb-2">Install iOS Shortcut</h3>
             <ol class="list-decimal list-inside space-y-2 mb-4">
                 <li>Open Settings > Shortcuts</li>
                 <li>Enable "Allow Untrusted Shortcuts"</li>
                 <li><a href="{}" class="btn btn-primary">Click here to install shortcut</a></li>
             </ol>
         </div>
     "#,
        deep_link
    ))
    .into_response())
}

// pub async fn serve_shortcut_template(
//     Query(params): Query<HashMap<String, String>>,
// ) -> impl IntoResponse {
//     let api_key = params.get("key").cloned().unwrap_or_default();

//     // Create the shortcut structure as a plist Value
//     let shortcut = Value::Dictionary(plist::Dictionary::from_iter([
//         (
//             "WFWorkflowActions".into(),
//             Value::Array(vec![
//                 // Text input action
//                 Value::Dictionary(plist::Dictionary::from_iter([
//                     (
//                         "WFWorkflowActionIdentifier".into(),
//                         Value::String("is.workflow.actions.ask".into()),
//                     ),
//                     (
//                         "WFWorkflowActionParameters".into(),
//                         Value::Dictionary(plist::Dictionary::from_iter([
//                             (
//                                 "WFAskActionPrompt".into(),
//                                 Value::String("Enter content or notes".into()),
//                             ),
//                             ("WFInputType".into(), Value::String("Text".into())),
//                         ])),
//                     ),
//                 ])),
//                 // File picker action
//                 Value::Dictionary(plist::Dictionary::from_iter([
//                     (
//                         "WFWorkflowActionIdentifier".into(),
//                         Value::String("is.workflow.actions.documentpicker".into()),
//                     ),
//                     (
//                         "WFWorkflowActionParameters".into(),
//                         Value::Dictionary(plist::Dictionary::from_iter([(
//                             "WFAllowMultipleSelection".into(),
//                             Value::Boolean(true),
//                         )])),
//                     ),
//                 ])),
//                 // API request action
//                 Value::Dictionary(plist::Dictionary::from_iter([
//                     (
//                         "WFWorkflowActionIdentifier".into(),
//                         Value::String("is.workflow.actions.downloadurl".into()),
//                     ),
//                     (
//                         "WFWorkflowActionParameters".into(),
//                         Value::Dictionary(plist::Dictionary::from_iter([
//                             ("Method".into(), Value::String("POST".into())),
//                             (
//                                 "URL".into(),
//                                 Value::String("http://your-api-endpoint.com/api/v2/ingress".into()),
//                             ),
//                             (
//                                 "Advanced".into(),
//                                 Value::Dictionary(plist::Dictionary::from_iter([(
//                                     "Headers".into(),
//                                     Value::Dictionary(plist::Dictionary::from_iter([(
//                                         "X-API-Key".into(),
//                                         Value::String(api_key),
//                                     )])),
//                                 )])),
//                             ),
//                         ])),
//                     ),
//                 ])),
//             ]),
//         ),
//         (
//             "WFWorkflowName".into(),
//             Value::String("Share to Your App".into()),
//         ),
//         (
//             "WFWorkflowTypes".into(),
//             Value::Array(vec![
//                 Value::String("NCWidget".into()),
//                 Value::String("WatchKit".into()),
//                 Value::String("QuickActions".into()),
//             ]),
//         ),
//     ]));

//     // Create a buffer to write the binary plist
//     let mut buffer = Cursor::new(Vec::new());
//     plist::to_writer_binary(&mut buffer, &shortcut).unwrap();
//     // ...

//     Response::builder()
//         .header("Content-Type", "application/x-ios-workflow")
//         .header(
//             "Content-Disposition",
//             "attachment; filename=\"share_to_app.shortcut\"",
//         )
//         .body(buffer.into_inner())
//         .unwrap()
// }
