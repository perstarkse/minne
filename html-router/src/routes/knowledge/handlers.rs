use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Form,
};
use plotly::{
    common::{Line, Marker, Mode},
    layout::{Axis, Camera, LayoutScene, ProjectionType},
    Layout, Plot, Scatter3D,
};
use serde::{Deserialize, Serialize};

use common::storage::types::{
    knowledge_entity::{KnowledgeEntity, KnowledgeEntityType},
    knowledge_relationship::KnowledgeRelationship,
    user::User,
};

use crate::{
    html_state::HtmlState,
    middlewares::{
        auth_middleware::RequireUser,
        response_middleware::{HtmlError, TemplateResponse},
    },
};

pub async fn show_knowledge_page(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    #[derive(Serialize)]
    pub struct KnowledgeBaseData {
        entities: Vec<KnowledgeEntity>,
        relationships: Vec<KnowledgeRelationship>,
        user: User,
        plot_html: String,
    }

    let entities = User::get_knowledge_entities(&user.id, &state.db).await?;

    let relationships = User::get_knowledge_relationships(&user.id, &state.db).await?;

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
            .hover_template(format!(
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

    Ok(TemplateResponse::new_template(
        "knowledge/base.html",
        KnowledgeBaseData {
            entities,
            relationships,
            user,
            plot_html: html,
        },
    ))
}

pub async fn show_edit_knowledge_entity_form(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    #[derive(Serialize)]
    pub struct EntityData {
        entity: KnowledgeEntity,
        entity_types: Vec<String>,
        user: User,
    }

    // Get entity types
    let entity_types: Vec<String> = KnowledgeEntityType::variants()
        .iter()
        .map(|s| s.to_string())
        .collect();

    // Get the entity and validate ownership
    let entity = User::get_and_validate_knowledge_entity(&id, &user.id, &state.db).await?;

    Ok(TemplateResponse::new_template(
        "knowledge/edit_knowledge_entity_modal.html",
        EntityData {
            entity,
            user,
            entity_types,
        },
    ))
}

#[derive(Debug, Deserialize)]
pub struct PatchKnowledgeEntityParams {
    pub id: String,
    pub name: String,
    pub entity_type: String,
    pub description: String,
}

#[derive(Serialize)]
pub struct EntityListData {
    entities: Vec<KnowledgeEntity>,
    user: User,
}

pub async fn patch_knowledge_entity(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Form(form): Form<PatchKnowledgeEntityParams>,
) -> Result<impl IntoResponse, HtmlError> {
    // Get the existing entity and validate that the user is allowed
    User::get_and_validate_knowledge_entity(&form.id, &user.id, &state.db).await?;

    let entity_type: KnowledgeEntityType = KnowledgeEntityType::from(form.entity_type);

    // Update the entity
    KnowledgeEntity::patch(
        &form.id,
        &form.name,
        &form.description,
        &entity_type,
        &state.db,
        &state.openai_client,
    )
    .await?;

    // Get updated list of entities
    let entities = User::get_knowledge_entities(&user.id, &state.db).await?;

    // Render updated list
    Ok(TemplateResponse::new_template(
        "knowledge/entity_list.html",
        EntityListData { entities, user },
    ))
}

pub async fn delete_knowledge_entity(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    // Get the existing entity and validate that the user is allowed
    User::get_and_validate_knowledge_entity(&id, &user.id, &state.db).await?;

    // Delete the entity
    state.db.delete_item::<KnowledgeEntity>(&id).await?;

    // Get updated list of entities
    let entities = User::get_knowledge_entities(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_template(
        "knowledge/entity_list.html",
        EntityListData { entities, user },
    ))
}

#[derive(Serialize)]
pub struct RelationshipTableData {
    entities: Vec<KnowledgeEntity>,
    relationships: Vec<KnowledgeRelationship>,
}

pub async fn delete_knowledge_relationship(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HtmlError> {
    // GOTTA ADD AUTH VALIDATION

    KnowledgeRelationship::delete_relationship_by_id(&id, &state.db).await?;

    let entities = User::get_knowledge_entities(&user.id, &state.db).await?;

    let relationships = User::get_knowledge_relationships(&user.id, &state.db).await?;

    // Render updated list
    Ok(TemplateResponse::new_template(
        "knowledge/relationship_table.html",
        RelationshipTableData {
            entities,
            relationships,
        },
    ))
}

#[derive(Deserialize)]
pub struct SaveKnowledgeRelationshipInput {
    pub in_: String,
    pub out: String,
    pub relationship_type: String,
}

pub async fn save_knowledge_relationship(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Form(form): Form<SaveKnowledgeRelationshipInput>,
) -> Result<impl IntoResponse, HtmlError> {
    // Construct relationship
    let relationship = KnowledgeRelationship::new(
        form.in_,
        form.out,
        user.id.clone(),
        "manual".into(),
        form.relationship_type,
    );

    relationship.store_relationship(&state.db).await?;

    let entities = User::get_knowledge_entities(&user.id, &state.db).await?;

    let relationships = User::get_knowledge_relationships(&user.id, &state.db).await?;

    // Render updated list
    Ok(TemplateResponse::new_template(
        "knowledge/relationship_table.html",
        RelationshipTableData {
            entities,
            relationships,
        },
    ))
}
