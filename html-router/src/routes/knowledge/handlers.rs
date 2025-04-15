use std::collections::{HashMap, VecDeque};

use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Form,
};
use axum_htmx::{HxBoosted, HxRequest};
use plotly::{
    common::{Line, Marker, Mode},
    layout::{Axis, LayoutScene},
    Layout, Plot, Scatter, Scatter3D,
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

#[derive(Deserialize, Default)]
pub struct FilterParams {
    entity_type: Option<String>,
    content_category: Option<String>,
}

#[derive(Serialize)]
pub struct KnowledgeBaseData {
    entities: Vec<KnowledgeEntity>,
    relationships: Vec<KnowledgeRelationship>,
    user: User,
    plot_html: String,
    entity_types: Vec<String>,
    content_categories: Vec<String>,
    selected_entity_type: Option<String>,
    selected_content_category: Option<String>,
}

pub async fn show_knowledge_page(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Query(mut params): Query<FilterParams>,
    HxRequest(is_htmx): HxRequest,
    HxBoosted(is_boosted): HxBoosted,
) -> Result<impl IntoResponse, HtmlError> {
    // Normalize filters
    params.entity_type = params.entity_type.take().filter(|s| !s.trim().is_empty());
    params.content_category = params
        .content_category
        .take()
        .filter(|s| !s.trim().is_empty());

    // Load relevant data
    let entity_types = User::get_entity_types(&user.id, &state.db).await?;
    let content_categories = User::get_user_categories(&user.id, &state.db).await?;

    // Load entities based on filters
    let entities = match &params.content_category {
        Some(cat) => {
            User::get_knowledge_entities_by_content_category(&user.id, cat, &state.db).await?
        }
        None => match &params.entity_type {
            Some(etype) => User::get_knowledge_entities_by_type(&user.id, etype, &state.db).await?,
            None => User::get_knowledge_entities(&user.id, &state.db).await?,
        },
    };

    let relationships = User::get_knowledge_relationships(&user.id, &state.db).await?;
    let plot_html = get_plot_html(&entities, &relationships)?;

    let kb_data = KnowledgeBaseData {
        entities,
        relationships,
        user,
        plot_html,
        entity_types,
        content_categories,
        selected_entity_type: params.entity_type.clone(),
        selected_content_category: params.content_category.clone(),
    };

    // Determine response type:
    // If it is an HTMX request but NOT a boosted navigation, send partial update (main block only)
    // Otherwise send full page including navbar/base for direct and boosted reloads
    if is_htmx && !is_boosted {
        // Partial update (just main block)
        Ok(TemplateResponse::new_partial(
            "knowledge/base.html",
            "main",
            &kb_data,
        ))
    } else {
        // Full page (includes navbar etc.)
        Ok(TemplateResponse::new_template(
            "knowledge/base.html",
            kb_data,
        ))
    }
}

fn get_plot_html(
    entities: &[KnowledgeEntity],
    relationships: &[KnowledgeRelationship],
) -> Result<String, HtmlError> {
    if entities.is_empty() {
        return Ok(String::new());
    }

    let id_to_idx: HashMap<_, _> = entities
        .iter()
        .enumerate()
        .map(|(i, e)| (e.id.clone(), i))
        .collect();

    // Build adjacency list
    let mut graph: Vec<Vec<usize>> = vec![Vec::new(); entities.len()];
    for rel in relationships {
        if let (Some(&from_idx), Some(&to_idx)) = (id_to_idx.get(&rel.out), id_to_idx.get(&rel.in_))
        {
            graph[from_idx].push(to_idx);
            graph[to_idx].push(from_idx);
        }
    }

    // Find clusters (connected components)
    let mut visited = vec![false; entities.len()];
    let mut clusters: Vec<Vec<usize>> = Vec::new();

    for i in 0..entities.len() {
        if !visited[i] {
            let mut queue = VecDeque::new();
            let mut cluster = Vec::new();
            queue.push_back(i);
            visited[i] = true;
            while let Some(node) = queue.pop_front() {
                cluster.push(node);
                for &nbr in &graph[node] {
                    if !visited[nbr] {
                        visited[nbr] = true;
                        queue.push_back(nbr);
                    }
                }
            }
            clusters.push(cluster);
        }
    }

    // Layout params
    let cluster_spacing = 20.0; // Distance between clusters
    let node_spacing = 3.0; // Distance between nodes within cluster

    // Arrange clusters on a Fibonacci sphere (uniform 3D positioning on unit sphere)
    let cluster_count = clusters.len();
    let golden_angle = std::f64::consts::PI * (3.0 - (5.0f64).sqrt());

    // Will hold final positions of nodes: (x,y,z)
    let mut nodes_pos = vec![(0.0f64, 0.0f64, 0.0f64); entities.len()];

    for (i, cluster) in clusters.iter().enumerate() {
        // Position cluster center on unit sphere scaled by cluster_spacing
        let theta = golden_angle * i as f64;
        let z = 1.0 - (2.0 * i as f64 + 1.0) / cluster_count as f64;
        let radius = (1.0 - z * z).sqrt();

        let cluster_center = (
            radius * theta.cos() * cluster_spacing,
            radius * theta.sin() * cluster_spacing,
            z * cluster_spacing,
        );

        // Layout nodes within cluster as small 3D grid (cube)
        // Calculate cube root to determine grid side length
        let cluster_size = cluster.len();
        let side_len = (cluster_size as f64).cbrt().ceil() as usize;

        for (pos_in_cluster, &node_idx) in cluster.iter().enumerate() {
            let x_in_cluster = (pos_in_cluster % side_len) as f64;
            let y_in_cluster = ((pos_in_cluster / side_len) % side_len) as f64;
            let z_in_cluster = (pos_in_cluster / (side_len * side_len)) as f64;

            nodes_pos[node_idx] = (
                cluster_center.0 + x_in_cluster * node_spacing,
                cluster_center.1 + y_in_cluster * node_spacing,
                cluster_center.2 + z_in_cluster * node_spacing,
            );
        }
    }

    let (node_x, node_y, node_z): (Vec<_>, Vec<_>, Vec<_>) = nodes_pos.iter().cloned().unzip3();

    // Nodes trace
    let nodes_trace = Scatter3D::new(node_x, node_y, node_z)
        .mode(Mode::Markers)
        .marker(Marker::new().size(8).color("#1f77b4"))
        .text_array(
            entities
                .iter()
                .map(|e| e.description.clone())
                .collect::<Vec<_>>(),
        )
        .hover_template("Entity: %{text}<extra></extra>");

    // Edges traces
    let mut plot = Plot::new();
    for rel in relationships {
        if let (Some(&from_idx), Some(&to_idx)) = (id_to_idx.get(&rel.out), id_to_idx.get(&rel.in_))
        {
            let edge_x = vec![nodes_pos[from_idx].0, nodes_pos[to_idx].0];
            let edge_y = vec![nodes_pos[from_idx].1, nodes_pos[to_idx].1];
            let edge_z = vec![nodes_pos[from_idx].2, nodes_pos[to_idx].2];

            let edge_trace = Scatter3D::new(edge_x, edge_y, edge_z)
                .mode(Mode::Lines)
                .line(Line::new().color("#888").width(2.0))
                .hover_template(format!(
                    "Relationship: {}<extra></extra>",
                    rel.metadata.relationship_type
                ))
                .show_legend(false);
            plot.add_trace(edge_trace);
        }
    }

    plot.add_trace(nodes_trace);

    // Layout scene configuration
    let layout = Layout::new()
        .scene(
            LayoutScene::new()
                .x_axis(Axis::new().visible(false))
                .y_axis(Axis::new().visible(false))
                .z_axis(Axis::new().visible(false))
                .camera(
                    plotly::layout::Camera::new()
                        .projection(plotly::layout::ProjectionType::Perspective.into())
                        .eye((2.0, 2.0, 2.0).into()),
                ),
        )
        .show_legend(false)
        .paper_background_color("rgba(255,255,255,0)")
        .plot_background_color("rgba(255,255,255,0)");

    plot.set_layout(layout);

    Ok(plot.to_html())
}

// Small utility to unzip tuple3 vectors from iterators (add this helper)
trait Unzip3<A, B, C> {
    fn unzip3(self) -> (Vec<A>, Vec<B>, Vec<C>);
}
impl<I, A, B, C> Unzip3<A, B, C> for I
where
    I: Iterator<Item = (A, B, C)>,
{
    fn unzip3(self) -> (Vec<A>, Vec<B>, Vec<C>) {
        let (mut va, mut vb, mut vc) = (Vec::new(), Vec::new(), Vec::new());
        for (a, b, c) in self {
            va.push(a);
            vb.push(b);
            vc.push(c);
        }
        (va, vb, vc)
    }
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
    entity_types: Vec<String>,
    content_categories: Vec<String>,
    selected_entity_type: Option<String>,
    selected_content_category: Option<String>,
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

    // Get entity types
    let entity_types = User::get_entity_types(&user.id, &state.db).await?;

    // Get content categories
    let content_categories = User::get_user_categories(&user.id, &state.db).await?;

    // Render updated list
    Ok(TemplateResponse::new_template(
        "knowledge/entity_list.html",
        EntityListData {
            entities,
            user,
            entity_types,
            content_categories,
            selected_entity_type: None,
            selected_content_category: None,
        },
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

    // Get entity types
    let entity_types = User::get_entity_types(&user.id, &state.db).await?;

    // Get content categories
    let content_categories = User::get_user_categories(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_template(
        "knowledge/entity_list.html",
        EntityListData {
            entities,
            user,
            entity_types,
            content_categories,
            selected_entity_type: None,
            selected_content_category: None,
        },
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
