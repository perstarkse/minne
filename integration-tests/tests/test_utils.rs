use common::storage::{
    db::SurrealDbClient,
    types::{
        knowledge_entity::KnowledgeEntity,
        user::User,
        system_settings::SystemSettings,
    },
};
use std::sync::Arc;
use uuid::Uuid;
use chrono::Utc;

/// Sets up an in-memory test database with migrations applied
pub async fn setup_test_database() -> Arc<SurrealDbClient> {
    let namespace = "test_ns";
    let database = Uuid::new_v4().to_string();
    
    let db = SurrealDbClient::memory(namespace, &database)
        .await
        .expect("Failed to start in-memory surrealdb");

    db.apply_migrations()
        .await
        .expect("Failed to setup the migrations");
    
    // Create default system settings
    let default_settings = SystemSettings::default();
    db.create_item(&default_settings)
        .await
        .expect("Failed to create default system settings");

    Arc::new(db)
}

/// Creates a test user with API key
pub async fn create_test_user(db: &SurrealDbClient) -> User {
    let user = User {
        id: format!("user:{}", Uuid::new_v4()),
        username: "testuser".to_string(),
        email: "test@example.com".to_string(),
        password_hash: "dummy_hash".to_string(),
        api_key: Some(Uuid::new_v4().to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    
    db.create_item(&user)
        .await
        .expect("Failed to create test user");
    
    user
}

/// Creates mock configuration for testing
pub fn create_mock_config() -> common::utils::config::AppConfig {
    common::utils::config::AppConfig {
        surrealdb_address: "memory".to_string(),
        surrealdb_username: "test".to_string(),
        surrealdb_password: "test".to_string(),
        surrealdb_database: "test".to_string(),
        surrealdb_namespace: "test".to_string(),
        openai_api_key: "test-key".to_string(),
        openai_base_url: "http://localhost:11434/v1".to_string(),
        http_port: 3000,
        data_dir: "/tmp/minne_test".to_string(),
    }
}

/// Creates test knowledge entities for testing search functionality
pub async fn create_test_knowledge_entities(db: &SurrealDbClient, user_id: &str) {
    let entities = vec![
        KnowledgeEntity {
            id: format!("knowledge_entity:{}", Uuid::new_v4()),
            entity_name: "Machine Learning".to_string(),
            entity_description: "A subset of artificial intelligence that involves training algorithms".to_string(),
            source_id: format!("source:{}", Uuid::new_v4()),
            user_id: user_id.to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            embedding: vec![0.1; 1536], // Mock embedding vector
        },
        KnowledgeEntity {
            id: format!("knowledge_entity:{}", Uuid::new_v4()),
            entity_name: "Neural Networks".to_string(),
            entity_description: "Computing systems inspired by biological neural networks".to_string(),
            source_id: format!("source:{}", Uuid::new_v4()),
            user_id: user_id.to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            embedding: vec![0.2; 1536], // Mock embedding vector
        },
    ];
    
    for entity in entities {
        db.create_item(&entity)
            .await
            .expect("Failed to create test knowledge entity");
    }
}