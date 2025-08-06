use axum::http::StatusCode;
use axum_test::TestServer;
use common::storage::{
    db::SurrealDbClient,
    types::{
        knowledge_entity::KnowledgeEntity, 
        user::User,
        system_settings::SystemSettings,
    },
};
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use uuid::Uuid;

mod test_utils;
use test_utils::*;

/// Integration tests for Minne's core features
/// These tests validate database operations and basic functionality

#[tokio::test]
async fn test_database_setup() {
    let db = setup_test_database().await;
    
    // Test basic database operations
    let settings = SystemSettings::default();
    db.create_item(&settings)
        .await
        .expect("Failed to create system settings");
    
    // Verify we can read back the settings
    let retrieved: Option<SystemSettings> = db.get_item("system_settings:default")
        .await
        .expect("Failed to retrieve system settings");
    
    assert!(retrieved.is_some());
}

#[tokio::test]
async fn test_user_creation() {
    let db = setup_test_database().await;
    let user = create_test_user(&db).await;
    
    // Verify user was created correctly
    assert!(!user.id.is_empty());
    assert!(user.api_key.is_some());
    assert_eq!(user.username, "testuser");
    assert_eq!(user.email, "test@example.com");
    
    // Verify we can retrieve the user from database
    let retrieved_user: Option<User> = db.get_item(&user.id)
        .await
        .expect("Failed to retrieve user");
    
    assert!(retrieved_user.is_some());
    let retrieved = retrieved_user.unwrap();
    assert_eq!(retrieved.id, user.id);
    assert_eq!(retrieved.username, user.username);
}

#[tokio::test]
async fn test_knowledge_entity_operations() {
    let db = setup_test_database().await;
    let user = create_test_user(&db).await;
    
    // Create test knowledge entities
    create_test_knowledge_entities(&db, &user.id).await;
    
    // Verify entities were created
    let entities: Vec<KnowledgeEntity> = db.query("SELECT * FROM knowledge_entity")
        .await
        .expect("Failed to query knowledge entities")
        .take(0)
        .expect("Failed to take query results");
    
    assert_eq!(entities.len(), 2);
    assert!(entities.iter().any(|e| e.entity_name == "Machine Learning"));
    assert!(entities.iter().any(|e| e.entity_name == "Neural Networks"));
}

#[tokio::test]
async fn test_basic_api_router_setup() {
    let db = setup_test_database().await;
    let user = create_test_user(&db).await;
    
    // Create API state
    let config = create_mock_config();
    let api_state = api_router::api_state::ApiState {
        db: db.clone(),
        config,
    };
    
    // Create basic API router
    let app = axum::Router::new()
        .nest("/api/v1", api_router::api_routes_v1(&api_state));
    
    let test_server = TestServer::new(app).unwrap();
    
    // Test categories endpoint with proper API key
    let response = test_server
        .get("/api/v1/categories")
        .add_header("x-api-key", &user.api_key.unwrap())
        .await;
    
    // Should return OK (even if empty categories)
    assert_eq!(response.status_code(), StatusCode::OK);
}

#[tokio::test]
async fn test_ingestion_api_unauthorized() {
    let db = setup_test_database().await;
    let _user = create_test_user(&db).await;
    
    let config = create_mock_config();
    let api_state = api_router::api_state::ApiState {
        db: db.clone(),
        config,
    };
    
    let app = axum::Router::new()
        .nest("/api/v1", api_router::api_routes_v1(&api_state));
    
    let test_server = TestServer::new(app).unwrap();
    
    // Test unauthorized access to ingestion endpoint
    let response = test_server
        .post("/api/v1/ingress")
        .multipart(
            axum_test::multipart::MultipartForm::new()
                .add_text("content", "This should fail")
                .add_text("context", "Unauthorized test")
                .add_text("category", "test"),
        )
        .await;
    
    // Should return unauthorized
    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_ingestion_api_with_auth() {
    let db = setup_test_database().await;
    let user = create_test_user(&db).await;
    
    let config = create_mock_config();
    let api_state = api_router::api_state::ApiState {
        db: db.clone(),
        config,
    };
    
    let app = axum::Router::new()
        .nest("/api/v1", api_router::api_routes_v1(&api_state));
    
    let test_server = TestServer::new(app).unwrap();
    
    // Test authorized access to ingestion endpoint
    let response = test_server
        .post("/api/v1/ingress")
        .add_header("x-api-key", &user.api_key.unwrap())
        .multipart(
            axum_test::multipart::MultipartForm::new()
                .add_text("content", "This is test content about machine learning")
                .add_text("context", "Testing ingestion pipeline")
                .add_text("category", "test"),
        )
        .await;
    
    // Should return OK (accepts the request)
    assert_eq!(response.status_code(), StatusCode::OK);
}

#[tokio::test]
async fn test_data_persistence() {
    let db = setup_test_database().await;
    let user = create_test_user(&db).await;
    
    // Create and store some data
    let entity = KnowledgeEntity {
        id: format!("knowledge_entity:{}", Uuid::new_v4()),
        entity_name: "Test Entity".to_string(),
        entity_description: "A test entity for persistence testing".to_string(),
        source_id: format!("source:{}", Uuid::new_v4()),
        user_id: user.id.clone(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        embedding: vec![0.5; 1536],
    };
    
    let entity_id = entity.id.clone();
    db.create_item(&entity)
        .await
        .expect("Failed to create test entity");
    
    // Retrieve and verify the data
    let retrieved: Option<KnowledgeEntity> = db.get_item(&entity_id)
        .await
        .expect("Failed to retrieve entity");
    
    assert!(retrieved.is_some());
    let retrieved_entity = retrieved.unwrap();
    assert_eq!(retrieved_entity.id, entity_id);
    assert_eq!(retrieved_entity.entity_name, "Test Entity");
    assert_eq!(retrieved_entity.user_id, user.id);
}

#[tokio::test]
async fn test_multiple_users_isolation() {
    let db = setup_test_database().await;
    
    // Create two separate users
    let user1 = create_test_user(&db).await;
    let user2_id = format!("user:{}", Uuid::new_v4());
    let user2 = User {
        id: user2_id.clone(),
        username: "testuser2".to_string(),
        email: "test2@example.com".to_string(),
        password_hash: "dummy_hash2".to_string(),
        api_key: Some(Uuid::new_v4().to_string()),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    
    db.create_item(&user2)
        .await
        .expect("Failed to create second user");
    
    // Create entities for each user
    let entity1 = KnowledgeEntity {
        id: format!("knowledge_entity:{}", Uuid::new_v4()),
        entity_name: "User1 Entity".to_string(),
        entity_description: "Entity belonging to user 1".to_string(),
        source_id: format!("source:{}", Uuid::new_v4()),
        user_id: user1.id.clone(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        embedding: vec![0.1; 1536],
    };
    
    let entity2 = KnowledgeEntity {
        id: format!("knowledge_entity:{}", Uuid::new_v4()),
        entity_name: "User2 Entity".to_string(),
        entity_description: "Entity belonging to user 2".to_string(),
        source_id: format!("source:{}", Uuid::new_v4()),
        user_id: user2.id.clone(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        embedding: vec![0.2; 1536],
    };
    
    db.create_item(&entity1)
        .await
        .expect("Failed to create entity1");
    db.create_item(&entity2)
        .await
        .expect("Failed to create entity2");
    
    // Verify user isolation - each user should only see their own entities
    let user1_entities: Vec<KnowledgeEntity> = db
        .query("SELECT * FROM knowledge_entity WHERE user_id = $user_id")
        .bind(("user_id", &user1.id))
        .await
        .expect("Failed to query user1 entities")
        .take(0)
        .expect("Failed to take query results");
    
    let user2_entities: Vec<KnowledgeEntity> = db
        .query("SELECT * FROM knowledge_entity WHERE user_id = $user_id")
        .bind(("user_id", &user2.id))
        .await
        .expect("Failed to query user2 entities")
        .take(0)
        .expect("Failed to take query results");
    
    assert_eq!(user1_entities.len(), 1);
    assert_eq!(user2_entities.len(), 1);
    assert_eq!(user1_entities[0].entity_name, "User1 Entity");
    assert_eq!(user2_entities[0].entity_name, "User2 Entity");
}