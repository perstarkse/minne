use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect},
    Form,
};
use axum_session_auth::AuthSession;
use axum_session_surreal::SessionSurrealPool;
use plotly::{
    common::{Line, Marker, Mode},
    layout::{Axis, Camera, LayoutScene, ProjectionType},
    Layout, Plot, Scatter3D,
};
use surrealdb::{engine::any::Any, Surreal};
use tracing::info;

use crate::{
    error::{AppError, HtmlError},
    page_data,
    server::{routes::html::render_template, AppState},
    storage::{
        db::delete_item,
        types::{
            knowledge_entity::KnowledgeEntity, knowledge_relationship::KnowledgeRelationship,
            user::User,
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

    let mut plot = Plot::new();

    // Fibonacci sphere distribution
    let node_count = entities.len();
    let golden_ratio = (1.0 + 5.0_f64.sqrt()) / 2.0;
    let node_positions: Vec<(f64, f64, f64)> = (0..node_count)
        .map(|i| {
            let i = i as f64;
            let theta = 2.0 * std::f64::consts::PI * i / golden_ratio;
            let phi = (1.0 - 2.0 * (i + 0.5) / node_count as f64).acos();
            let x = phi.sin() * theta.cos();
            let y = phi.sin() * theta.sin();
            let z = phi.cos();
            (x, y, z)
        })
        .collect();

    let node_x: Vec<f64> = node_positions.iter().map(|(x, _, _)| *x).collect();
    let node_y: Vec<f64> = node_positions.iter().map(|(_, y, _)| *y).collect();
    let node_z: Vec<f64> = node_positions.iter().map(|(_, _, z)| *z).collect();

    // Nodes trace
    let nodes = Scatter3D::new(node_x.clone(), node_y.clone(), node_z.clone())
        .mode(Mode::Markers)
        .marker(Marker::new().size(8).color("#1f77b4"))
        .text_array(
            entities
                .iter()
                .map(|e| e.description.clone())
                .collect::<Vec<_>>(),
        )
        .hover_template("Entity: %{text}<br>");

    // Edges traces
    for rel in &relationships {
        let from_idx = entities.iter().position(|e| e.id == rel.out).unwrap_or(0);
        let to_idx = entities.iter().position(|e| e.id == rel.in_).unwrap_or(0);

        let edge_x = vec![node_x[from_idx], node_x[to_idx]];
        let edge_y = vec![node_y[from_idx], node_y[to_idx]];
        let edge_z = vec![node_z[from_idx], node_z[to_idx]];

        let edge_trace = Scatter3D::new(edge_x, edge_y, edge_z)
            .mode(Mode::Lines)
            .line(Line::new().color("#888").width(2.0))
            .hover_template(&format!(
                "Relationship: {}<br>",
                rel.metadata.relationship_type
            ))
            .show_legend(false);

        plot.add_trace(edge_trace);
    }
    plot.add_trace(nodes);

    // Layout
    let layout = Layout::new()
        .scene(
            LayoutScene::new()
                .x_axis(Axis::new().visible(false))
                .y_axis(Axis::new().visible(false))
                .z_axis(Axis::new().visible(false))
                .camera(
                    Camera::new()
                        .projection(ProjectionType::Perspective.into())
                        .eye((1.5, 1.5, 1.5).into()),
                ),
        )
        .show_legend(false)
        .paper_background_color("rbga(250,100,0,0)")
        .plot_background_color("rbga(0,0,0,0)");

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

#[derive(Serialize)]
pub struct EntityData {
    entity: KnowledgeEntity,
    user: User,
}

pub async fn show_edit_knowledge_entity_form(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/signin").into_response()),
    };

    // Get the entity and validate ownership
    let entity = User::get_and_validate_knowledge_entity(&id, &user.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let output = render_template(
        "knowledge/edit_knowledge_entity_modal.html",
        EntityData { entity, user },
        state.templates,
    )?;

    Ok(output.into_response())
}

#[derive(Serialize)]
pub struct EntityListData {
    entities: Vec<KnowledgeEntity>,
    user: User,
}

#[derive(Debug, Deserialize)]
pub struct PatchKnowledgeEntityParams {
    pub id: String,
    pub name: String,
    pub description: String,
}

pub async fn patch_knowledge_entity(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Form(form): Form<PatchKnowledgeEntityParams>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/signin").into_response()),
    };

    // Get the existing entity and validate that the user is allowed
    let existing_entity =
        User::get_and_validate_knowledge_entity(&form.id, &user.id, &state.surreal_db_client)
            .await
            .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    // Update the entity
    KnowledgeEntity::patch(
        &form.id,
        &form.name,
        &form.description,
        &existing_entity.entity_type,
        &state.surreal_db_client,
        &state.openai_client,
    )
    .await
    .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;

    // Get updated list of entities
    let entities = User::get_knowledge_entities(&user.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    // Render updated list
    let output = render_template(
        "knowledge/entity_list.html",
        EntityListData { entities, user },
        state.templates,
    )?;

    Ok(output.into_response())
}

pub async fn delete_knowledge_entity(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/signin").into_response()),
    };

    // Get the existing entity and validate that the user is allowed
    User::get_and_validate_knowledge_entity(&id, &user.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    // Delete the entity
    delete_item::<KnowledgeEntity>(&state.surreal_db_client, &id)
        .await
        .map_err(|e| HtmlError::new(AppError::from(e), state.templates.clone()))?;

    // Get updated list of entities
    let entities = User::get_knowledge_entities(&user.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    // Render updated list
    let output = render_template(
        "knowledge/entity_list.html",
        EntityListData { entities, user },
        state.templates,
    )?;

    Ok(output.into_response())
}

#[derive(Serialize)]
pub struct RelationshipTableData {
    entities: Vec<KnowledgeEntity>,
    relationships: Vec<KnowledgeRelationship>,
}

pub async fn delete_knowledge_relationship(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/signin").into_response()),
    };

    // GOTTA ADD AUTH VALIDATION

    KnowledgeRelationship::delete_relationship_by_id(&id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let entities = User::get_knowledge_entities(&user.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let relationships = User::get_knowledge_relationships(&user.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    // Render updated list
    let output = render_template(
        "knowledge/relationship_table.html",
        RelationshipTableData {
            entities,
            relationships,
        },
        state.templates,
    )?;

    Ok(output.into_response())
}

#[derive(Deserialize)]
pub struct SaveKnowledgeRelationshipInput {
    pub in_: String,
    pub out: String,
    pub relationship_type: String,
}

pub async fn save_knowledge_relationship(
    State(state): State<AppState>,
    auth: AuthSession<User, String, SessionSurrealPool<Any>, Surreal<Any>>,
    Form(form): Form<SaveKnowledgeRelationshipInput>,
) -> Result<impl IntoResponse, HtmlError> {
    // Early return if the user is not authenticated
    let user = match auth.current_user {
        Some(user) => user,
        None => return Ok(Redirect::to("/signin").into_response()),
    };

    // Construct relationship
    let relationship = KnowledgeRelationship::new(
        form.in_,
        form.out,
        user.id.clone(),
        "manual".into(),
        form.relationship_type,
    );

    relationship
        .store_relationship(&state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let entities = User::get_knowledge_entities(&user.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    let relationships = User::get_knowledge_relationships(&user.id, &state.surreal_db_client)
        .await
        .map_err(|e| HtmlError::new(e, state.templates.clone()))?;

    // Render updated list
    let output = render_template(
        "knowledge/relationship_table.html",
        RelationshipTableData {
            entities,
            relationships,
        },
        state.templates,
    )?;

    Ok(output.into_response())
}
