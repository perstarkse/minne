use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect},
};
use axum_session::Session;
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use plotly::{Configuration, Layout, Plot, Scatter};
use surrealdb::{engine::any::Any, Surreal};
use tokio::join;
use tracing::info;

use crate::{
    error::{AppError, HtmlError},
    page_data,
    server::{
        routes::html::{render_block, render_template},
        AppState,
    },
    storage::{
        db::{delete_item, get_item},
        types::{
            file_info::FileInfo, job::Job, knowledge_entity::KnowledgeEntity,
            knowledge_relationship::KnowledgeRelationship, text_chunk::TextChunk,
            text_content::TextContent, user::User,
        },
    },
};

page_data!(KnowledgeBaseData, "knowledge/base.html", {
    entities: Vec<KnowledgeEntity>,
    relationships: Vec<KnowledgeRelationship>,
    user: User,
    plot_html: String
});

pub async fn show_knowledge_page(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/signin").into_response()),
    };

    let entities = User::get_knowledge_entities(&user.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    info!("Got entities ok");

    let relationships = User::get_knowledge_relationships(&user.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    // In your handler function
    let mut plot = Plot::new();

    // Create node positions (you might want to use a proper layout algorithm)
    let node_x: Vec<f64> = entities.iter().enumerate().map(|(i, _)| i as f64).collect();
    let node_y: Vec<f64> = vec![0.0; entities.len()];
    let node_text: Vec<String> = entities.iter().map(|e| e.description.clone()).collect();

    // Add nodes
    let nodes = Scatter::new(node_x.clone(), node_y.clone())
        .mode(plotly::common::Mode::Markers)
        .text_array(node_text)
        .name("Entities")
        .hover_template("%{text}");

    // Add edges
    let mut edge_x = Vec::new();
    let mut edge_y = Vec::new();
    for rel in &relationships {
        let from_idx = entities.iter().position(|e| e.id == rel.out).unwrap_or(0);
        let to_idx = entities.iter().position(|e| e.id == rel.in_).unwrap_or(0);

        edge_x.extend_from_slice(&[from_idx as f64, to_idx as f64, std::f64::NAN]);
        edge_y.extend_from_slice(&[0.0, 0.0, std::f64::NAN]);
    }

    let edges = Scatter::new(edge_x, edge_y)
        .mode(plotly::common::Mode::Lines)
        .name("Relationships");

    plot.add_trace(edges);
    plot.add_trace(nodes);

    let layout = Layout::new()
        .title("Knowledge Graph")
        .show_legend(false)
        .height(600);

    plot.set_layout(layout);

    // Convert to HTML
    let html = plot.to_html();

    let output = render_template(
        KnowledgeBaseData::template_name(),
        KnowledgeBaseData {
            entities,
            relationships,
            user,
            plot_html: html,
        },
        state.templates,
    )?;

    Ok(output.into_response())
}
