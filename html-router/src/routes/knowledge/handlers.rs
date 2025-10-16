use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt;

use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Form, Json,
};
use axum_htmx::{HxBoosted, HxRequest};
use serde::{
    de::{self, Deserializer, MapAccess, Visitor},
    Deserialize, Serialize,
};

use common::{
    error::AppError,
    storage::types::{
        conversation::Conversation,
        knowledge_entity::{KnowledgeEntity, KnowledgeEntityType},
        knowledge_relationship::KnowledgeRelationship,
        user::User,
    },
    utils::embedding::generate_embedding,
};
use composite_retrieval::{retrieve_entities, RetrievedEntity};
use tracing::debug;
use uuid::Uuid;

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
const DEFAULT_RELATIONSHIP_TYPE: &str = "relates_to";
const MAX_RELATIONSHIP_SUGGESTIONS: usize = 10;
const SUGGESTION_MIN_SCORE: f32 = 0.5;

#[derive(Deserialize, Default)]
pub struct FilterParams {
    entity_type: Option<String>,
    content_category: Option<String>,
    page: Option<usize>,
}

pub async fn show_new_knowledge_entity_form(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
) -> Result<impl IntoResponse, HtmlError> {
    let entity_types: Vec<String> = KnowledgeEntityType::variants()
        .iter()
        .map(|&s| s.to_owned())
        .collect();

    let existing_entities = User::get_knowledge_entities(&user.id, &state.db).await?;
    let empty_selected: HashSet<String> = HashSet::new();
    let empty_scores: HashMap<String, f32> = HashMap::new();
    let relationship_options =
        build_relationship_options(existing_entities, &empty_selected, &empty_scores);

    Ok(TemplateResponse::new_template(
        "knowledge/new_knowledge_entity_modal.html",
        NewEntityModalData {
            entity_types,
            relationship_list: RelationshipListData {
                relationship_options,
                relationship_type: DEFAULT_RELATIONSHIP_TYPE.to_string(),
                suggestion_count: 0,
            },
        },
    ))
}

pub async fn create_knowledge_entity(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Form(form): Form<CreateKnowledgeEntityParams>,
) -> Result<impl IntoResponse, HtmlError> {
    let name = form.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Validation("Name is required".into()).into());
    }

    let description = form.description.trim().to_string();
    let entity_type = KnowledgeEntityType::from(form.entity_type.trim().to_string());

    let embedding_input =
        format!("name: {name}, description: {description}, type: {entity_type:?}");
    let embedding = generate_embedding(&state.openai_client, &embedding_input, &state.db).await?;

    let source_id = format!("manual::{}", Uuid::new_v4());
    let new_entity = KnowledgeEntity::new(
        source_id,
        name.clone(),
        description.clone(),
        entity_type,
        None,
        embedding,
        user.id.clone(),
    );

    state.db.store_item(new_entity.clone()).await?;

    let relationship_type = form
        .relationship_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_RELATIONSHIP_TYPE)
        .to_string();

    debug!("form: {:?}", form);
    if !form.relationship_ids.is_empty() {
        let existing_entities = User::get_knowledge_entities(&user.id, &state.db).await?;
        let valid_ids: HashSet<String> = existing_entities
            .into_iter()
            .map(|entity| entity.id)
            .collect();
        let mut unique_ids: HashSet<String> = HashSet::new();

        for target_id in form.relationship_ids {
            if target_id == new_entity.id {
                continue;
            }
            if !valid_ids.contains(&target_id) {
                continue;
            }
            if !unique_ids.insert(target_id.clone()) {
                continue;
            }

            let relationship = KnowledgeRelationship::new(
                new_entity.id.clone(),
                target_id,
                user.id.clone(),
                format!("manual::{}", new_entity.id),
                relationship_type.clone(),
            );
            relationship.store_relationship(&state.db).await?;
        }
    }

    let default_params = FilterParams::default();
    let kb_data = build_knowledge_base_data(&state, &user, &default_params).await?;
    Ok(TemplateResponse::new_partial(
        "knowledge/base.html",
        "main",
        kb_data,
    ))
}

pub async fn suggest_knowledge_relationships(
    State(state): State<HtmlState>,
    RequireUser(user): RequireUser,
    Form(form): Form<SuggestRelationshipsParams>,
) -> Result<impl IntoResponse, HtmlError> {
    let entity_lookup: HashMap<String, KnowledgeEntity> =
        User::get_knowledge_entities(&user.id, &state.db)
            .await?
            .into_iter()
            .map(|entity| (entity.id.clone(), entity))
            .collect();

    let mut selected_ids: HashSet<String> = form
        .relationship_ids
        .into_iter()
        .filter(|id| entity_lookup.contains_key(id))
        .collect();

    let mut suggestion_scores: HashMap<String, f32> = HashMap::new();

    let mut query_parts = Vec::new();
    if let Some(name) = form
        .name
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        query_parts.push(name.to_string());
    }
    if let Some(description) = form
        .description
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        query_parts.push(description.to_string());
    }

    if !query_parts.is_empty() {
        let query = query_parts.join(" ");
        if let Ok(results) =
            retrieve_entities(&state.db, &state.openai_client, &query, &user.id).await
        {
            for RetrievedEntity { entity, score, .. } in results {
                if suggestion_scores.len() >= MAX_RELATIONSHIP_SUGGESTIONS {
                    break;
                }
                if score.is_nan() || score < SUGGESTION_MIN_SCORE {
                    continue;
                }
                if !entity_lookup.contains_key(&entity.id) {
                    continue;
                }
                suggestion_scores.insert(entity.id.clone(), score);
                selected_ids.insert(entity.id.clone());
            }
        }
    }

    let relationship_type = form
        .relationship_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_RELATIONSHIP_TYPE)
        .to_string();

    let entities: Vec<KnowledgeEntity> = entity_lookup.into_values().collect();
    let relationship_options =
        build_relationship_options(entities, &selected_ids, &suggestion_scores);

    Ok(TemplateResponse::new_template(
        "knowledge/relationship_selector.html",
        RelationshipListData {
            relationship_options,
            relationship_type,
            suggestion_count: suggestion_scores.len(),
        },
    ))
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

#[derive(Serialize)]
pub struct RelationshipOption {
    entity: KnowledgeEntity,
    is_selected: bool,
    is_suggested: bool,
    score: Option<f32>,
}

fn build_relationship_options(
    entities: Vec<KnowledgeEntity>,
    selected_ids: &HashSet<String>,
    suggestion_scores: &HashMap<String, f32>,
) -> Vec<RelationshipOption> {
    let mut options: Vec<RelationshipOption> = entities
        .into_iter()
        .map(|entity| {
            let id = entity.id.clone();
            let score = suggestion_scores.get(&id).copied();
            RelationshipOption {
                entity,
                is_selected: selected_ids.contains(&id),
                is_suggested: score.is_some(),
                score,
            }
        })
        .collect();

    options.sort_by(|a, b| match (a.is_suggested, b.is_suggested) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => match (a.score, b.score) {
            (Some(a_score), Some(b_score)) => {
                b_score.partial_cmp(&a_score).unwrap_or(Ordering::Equal)
            }
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => a
                .entity
                .name
                .to_lowercase()
                .cmp(&b.entity.name.to_lowercase()),
        },
    });

    options
}

async fn build_knowledge_base_data(
    state: &HtmlState,
    user: &User,
    params: &FilterParams,
) -> Result<KnowledgeBaseData, AppError> {
    let entity_types = User::get_entity_types(&user.id, &state.db).await?;
    let content_categories = User::get_user_categories(&user.id, &state.db).await?;

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
            format!("&{encoded}")
        }
    };

    let relationships = User::get_knowledge_relationships(&user.id, &state.db).await?;
    let entity_id_set: HashSet<String> = entities.iter().map(|e| e.id.clone()).collect();
    let relationships: Vec<KnowledgeRelationship> = relationships
        .into_iter()
        .filter(|rel| entity_id_set.contains(&rel.in_) && entity_id_set.contains(&rel.out))
        .collect();
    let conversation_archive = User::get_user_conversations(&user.id, &state.db).await?;

    Ok(KnowledgeBaseData {
        entities,
        visible_entities,
        relationships,
        user: user.clone(),
        entity_types,
        content_categories,
        selected_entity_type: params.entity_type.clone(),
        selected_content_category: params.content_category.clone(),
        conversation_archive,
        pagination,
        page_query,
    })
}

#[derive(Serialize)]
pub struct RelationshipListData {
    relationship_options: Vec<RelationshipOption>,
    relationship_type: String,
    suggestion_count: usize,
}

#[derive(Serialize)]
pub struct NewEntityModalData {
    entity_types: Vec<String>,
    relationship_list: RelationshipListData,
}

#[derive(Debug)]
pub struct CreateKnowledgeEntityParams {
    pub name: String,
    pub entity_type: String,
    pub description: String,
    pub relationship_type: Option<String>,
    pub relationship_ids: Vec<String>,
}

impl<'de> Deserialize<'de> for CreateKnowledgeEntityParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            Name,
            EntityType,
            Description,
            RelationshipType,
            #[serde(alias = "relationship_ids[]")]
            RelationshipIds,
        }

        struct ParamsVisitor;

        impl<'de> Visitor<'de> for ParamsVisitor {
            type Value = CreateKnowledgeEntityParams;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct CreateKnowledgeEntityParams")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut name: Option<String> = None;
                let mut entity_type: Option<String> = None;
                let mut description: Option<String> = None;
                let mut relationship_type: Option<String> = None;
                let mut relationship_ids: Vec<String> = Vec::new();

                while let Some(key) = map.next_key::<Field>()? {
                    match key {
                        Field::Name => {
                            if name.is_some() {
                                return Err(de::Error::duplicate_field("name"));
                            }
                            name = Some(map.next_value()?);
                        }
                        Field::EntityType => {
                            if entity_type.is_some() {
                                return Err(de::Error::duplicate_field("entity_type"));
                            }
                            entity_type = Some(map.next_value()?);
                        }
                        Field::Description => {
                            description = Some(map.next_value()?);
                        }
                        Field::RelationshipType => {
                            relationship_type = Some(map.next_value()?);
                        }
                        Field::RelationshipIds => {
                            let value: String = map.next_value()?;
                            let trimmed = value.trim();
                            if !trimmed.is_empty() {
                                relationship_ids.push(trimmed.to_owned());
                            }
                        }
                    }
                }

                let name = name.ok_or_else(|| de::Error::missing_field("name"))?;
                let entity_type =
                    entity_type.ok_or_else(|| de::Error::missing_field("entity_type"))?;
                let description = description.unwrap_or_default();
                let relationship_type = relationship_type
                    .map(|value: String| value.trim().to_owned())
                    .filter(|value| !value.is_empty());

                Ok(CreateKnowledgeEntityParams {
                    name,
                    entity_type,
                    description,
                    relationship_type,
                    relationship_ids,
                })
            }
        }

        const FIELDS: &[&str] = &[
            "name",
            "entity_type",
            "description",
            "relationship_type",
            "relationship_ids",
        ];

        deserializer.deserialize_struct("CreateKnowledgeEntityParams", FIELDS, ParamsVisitor)
    }
}

#[derive(Debug)]
pub struct SuggestRelationshipsParams {
    pub name: Option<String>,
    pub description: Option<String>,
    pub relationship_type: Option<String>,
    pub relationship_ids: Vec<String>,
}

impl<'de> Deserialize<'de> for SuggestRelationshipsParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            Name,
            Description,
            RelationshipType,
            EntityType,
            #[serde(alias = "relationship_ids[]")]
            RelationshipIds,
        }

        struct ParamsVisitor;

        impl<'de> Visitor<'de> for ParamsVisitor {
            type Value = SuggestRelationshipsParams;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct SuggestRelationshipsParams")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut name: Option<String> = None;
                let mut description: Option<String> = None;
                let mut relationship_type: Option<String> = None;
                let mut relationship_ids: Vec<String> = Vec::new();

                while let Some(key) = map.next_key::<Field>()? {
                    match key {
                        Field::Name => {
                            if name.is_some() {
                                return Err(de::Error::duplicate_field("name"));
                            }
                            let value: String = map.next_value()?;
                            let trimmed = value.trim();
                            if !trimmed.is_empty() {
                                name = Some(trimmed.to_owned());
                            }
                        }
                        Field::Description => {
                            let value: String = map.next_value()?;
                            let trimmed = value.trim();
                            if trimmed.is_empty() {
                                description = None;
                            } else {
                                description = Some(trimmed.to_owned());
                            }
                        }
                        Field::RelationshipType => {
                            let value: String = map.next_value()?;
                            let trimmed = value.trim();
                            if trimmed.is_empty() {
                                relationship_type = None;
                            } else {
                                relationship_type = Some(trimmed.to_owned());
                            }
                        }
                        Field::EntityType => {
                            map.next_value::<de::IgnoredAny>()?;
                        }
                        Field::RelationshipIds => {
                            let value: String = map.next_value()?;
                            let trimmed = value.trim();
                            if !trimmed.is_empty() {
                                relationship_ids.push(trimmed.to_owned());
                            }
                        }
                    }
                }

                Ok(SuggestRelationshipsParams {
                    name,
                    description,
                    relationship_type,
                    relationship_ids,
                })
            }
        }

        const FIELDS: &[&str] = &[
            "name",
            "description",
            "relationship_type",
            "entity_type",
            "relationship_ids",
        ];

        deserializer.deserialize_struct("SuggestRelationshipsParams", FIELDS, ParamsVisitor)
    }
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

    let kb_data = build_knowledge_base_data(&state, &user, &params).await?;

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
    for rel in &relationships {
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
    input.and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
            None
        } else {
            Some(trim_matching_quotes(trimmed).to_string())
        }
    })
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
        .map(|&s| s.to_owned())
        .collect();

    // Get the entity and validate ownership
    let entity = User::get_and_validate_knowledge_entity(&id, &user.id, &state.db).await?;

    Ok(TemplateResponse::new_template(
        "knowledge/edit_knowledge_entity_modal.html",
        EntityData {
            entity,
            entity_types,
            user,
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
