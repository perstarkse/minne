use std::collections::{HashMap, HashSet};

use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Form, Json,
};
use axum_htmx::{HxBoosted, HxRequest};
use serde::{Deserialize, Serialize};

use common::storage::types::{
    conversation::Conversation,
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
    utils::pagination::{paginate_items, Pagination},
};
use url::form_urlencoded;

const KNOWLEDGE_ENTITIES_PER_PAGE: usize = 12;

#[derive(Deserialize, Default)]
pub struct FilterParams {
    entity_type: Option<String>,
    content_category: Option<String>,
    page: Option<usize>,
}

#[derive(Serialize)]
pub struct KnowledgeBaseData {
    entities: Vec<KnowledgeEntity>,
    visible_entities: Vec<KnowledgeEntity>,
    relationships: Vec<KnowledgeRelationship>,
    user: User,
    entity_types: Vec<String>,
    content_categories: Vec<String>,
    selected_entity_type: Option<String>,
    selected_content_category: Option<String>,
    conversation_archive: Vec<Conversation>,
    pagination: Pagination,
    page_query: String,
}

pub async fn show_knowledge_page(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    HxRequest(is_htmx): HxRequest,
    HxBoosted(is_boosted): HxBoosted,
    Query(mut params): Query<FilterParams>,
) -> Result<impl IntoResponse, HtmlError> {
    // Normalize filters: treat empty or "none" as no filter
    params.entity_type = normalize_filter(params.entity_type.take());
    params.content_category = normalize_filter(params.content_category.take());

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

    let (visible_entities, pagination) =
        paginate_items(entities.clone(), params.page, KNOWLEDGE_ENTITIES_PER_PAGE);

    let page_query = {
        let mut serializer = form_urlencoded::Serializer::new(String::new());
        if let Some(entity_type) = params.entity_type.as_deref() {
            serializer.append_pair("entity_type", entity_type);
        }
        if let Some(content_category) = params.content_category.as_deref() {
            serializer.append_pair("content_category", content_category);
        }
        let encoded = serializer.finish();
        if encoded.is_empty() {
            String::new()
        } else {
            format!("&{}", encoded)
        }
    };

    let relationships = User::get_knowledge_relationships(&user.id, &state.db).await?;
    let entity_id_set: HashSet<String> = entities.iter().map(|e| e.id.clone()).collect();
    let relationships: Vec<KnowledgeRelationship> = relationships
        .into_iter()
        .filter(|rel| entity_id_set.contains(&rel.in_) && entity_id_set.contains(&rel.out))
        .collect();
    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    let kb_data = KnowledgeBaseData {
        entities,
        visible_entities,
        relationships,
        user,
        entity_types,
        content_categories,
        selected_entity_type: params.entity_type.clone(),
        selected_content_category: params.content_category.clone(),
        conversation_archive,
        pagination,
        page_query,
    };

    // Determine response type:
    // If it is an HTMX request but NOT a boosted navigation, send partial update (main block only)
    // Otherwise send full page including navbar/base for direct and boosted reloads
    if is_htmx && !is_boosted {
        Ok(TemplateResponse::new_partial(
            "knowledge/base.html",
            "main",
            &kb_data,
        ))
    } else {
        Ok(TemplateResponse::new_template(
            "knowledge/base.html",
            kb_data,
        ))
    }
}

#[derive(Serialize)]
pub struct GraphNode {
    pub id: String,
    pub name: String,
    pub entity_type: String,
    pub degree: usize,
}

#[derive(Serialize)]
pub struct GraphLink {
    pub source: String,
    pub target: String,
    pub relationship_type: String,
}

#[derive(Serialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub links: Vec<GraphLink>,
}

pub async fn get_knowledge_graph_json(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Query(mut params): Query<FilterParams>,
) -> Result<impl IntoResponse, HtmlError> {
    // Normalize filters: treat empty or "none" as no filter
    params.entity_type = normalize_filter(params.entity_type.take());
    params.content_category = normalize_filter(params.content_category.take());

    // Load entities based on filters
    let entities: Vec<KnowledgeEntity> = match &params.content_category {
        Some(cat) => {
            User::get_knowledge_entities_by_content_category(&user.id, cat, &state.db).await?
        }
        None => match &params.entity_type {
            Some(etype) => User::get_knowledge_entities_by_type(&user.id, etype, &state.db).await?,
            None => User::get_knowledge_entities(&user.id, &state.db).await?,
        },
    };

    // All relationships for user, then filter to those whose endpoints are in the set
    let relationships: Vec<KnowledgeRelationship> =
        User::get_knowledge_relationships(&user.id, &state.db).await?;

    let entity_ids: HashSet<String> = entities.iter().map(|e| e.id.clone()).collect();

    let mut degree_count: HashMap<String, usize> = HashMap::new();
    let mut links: Vec<GraphLink> = Vec::new();
    for rel in relationships.iter() {
        if entity_ids.contains(&rel.in_) && entity_ids.contains(&rel.out) {
            // undirected counting for degree
            *degree_count.entry(rel.in_.clone()).or_insert(0) += 1;
            *degree_count.entry(rel.out.clone()).or_insert(0) += 1;
            links.push(GraphLink {
                source: rel.out.clone(),
                target: rel.in_.clone(),
                relationship_type: rel.metadata.relationship_type.clone(),
            });
        }
    }

    let nodes: Vec<GraphNode> = entities
        .into_iter()
        .map(|e| GraphNode {
            id: e.id.clone(),
            name: e.name.clone(),
            entity_type: format!("{:?}", e.entity_type),
            degree: *degree_count.get(&e.id).unwrap_or(&0),
        })
        .collect();

    Ok(Json(GraphData { nodes, links }))
}
// Normalize filter parameters: convert empty strings or "none" (case-insensitive) to None
fn normalize_filter(input: Option<String>) -> Option<String> {
    match input {
        None => None,
        Some(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
                None
            } else {
                Some(trim_matching_quotes(trimmed).to_string())
            }
        }
    }
}

fn trim_matching_quotes(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &value[1..value.len() - 1];
        }
    }
    value
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
    visible_entities: Vec<KnowledgeEntity>,
    pagination: Pagination,
    user: User,
    entity_types: Vec<String>,
    content_categories: Vec<String>,
    selected_entity_type: Option<String>,
    selected_content_category: Option<String>,
    page_query: String,
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
    let (visible_entities, pagination) = paginate_items(
        User::get_knowledge_entities(&user.id, &state.db).await?,
        Some(1),
        KNOWLEDGE_ENTITIES_PER_PAGE,
    );

    // Get entity types
    let entity_types = User::get_entity_types(&user.id, &state.db).await?;

    // Get content categories
    let content_categories = User::get_user_categories(&user.id, &state.db).await?;

    // Render updated list
    Ok(TemplateResponse::new_template(
        "knowledge/entity_list.html",
        EntityListData {
            visible_entities,
            pagination,
            user,
            entity_types,
            content_categories,
            selected_entity_type: None,
            selected_content_category: None,
            page_query: String::new(),
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
    let (visible_entities, pagination) = paginate_items(
        User::get_knowledge_entities(&user.id, &state.db).await?,
        Some(1),
        KNOWLEDGE_ENTITIES_PER_PAGE,
    );

    // Get entity types
    let entity_types = User::get_entity_types(&user.id, &state.db).await?;

    // Get content categories
    let content_categories = User::get_user_categories(&user.id, &state.db).await?;

    Ok(TemplateResponse::new_template(
        "knowledge/entity_list.html",
        EntityListData {
            visible_entities,
            pagination,
            user,
            entity_types,
            content_categories,
            selected_entity_type: None,
            selected_content_category: None,
            page_query: String::new(),
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
    KnowledgeRelationship::delete_relationship_by_id(&id, &user.id, &state.db).await?;

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
