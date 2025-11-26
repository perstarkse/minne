# Defines the schema for the 'knowledge_entity' table.

DEFINE TABLE IF NOT EXISTS knowledge_entity SCHEMALESS;

# Standard fields
DEFINE FIELD IF NOT EXISTS created_at ON knowledge_entity TYPE datetime;
DEFINE FIELD IF NOT EXISTS updated_at ON knowledge_entity TYPE datetime;

# Custom fields from the KnowledgeEntity struct
DEFINE FIELD IF NOT EXISTS source_id ON knowledge_entity TYPE string;
DEFINE FIELD IF NOT EXISTS name ON knowledge_entity TYPE string;
DEFINE FIELD IF NOT EXISTS description ON knowledge_entity TYPE string;
# KnowledgeEntityType is an enum, store as string
DEFINE FIELD IF NOT EXISTS entity_type ON knowledge_entity TYPE string;
# metadata is Option<serde_json::Value>, store as object
DEFINE FIELD IF NOT EXISTS metadata ON knowledge_entity TYPE option<object>;

DEFINE FIELD IF NOT EXISTS user_id ON knowledge_entity TYPE string;

-- Indexes based on build_indexes and query patterns
-- HNSW index now defined on knowledge_entity_embedding table for better memory usage
-- DEFINE INDEX IF NOT EXISTS idx_embedding_entities ON knowledge_entity FIELDS embedding HNSW DIMENSION 1536;
DEFINE INDEX IF NOT EXISTS knowledge_entity_source_id_idx ON knowledge_entity FIELDS source_id;
DEFINE INDEX IF NOT EXISTS knowledge_entity_user_id_idx ON knowledge_entity FIELDS user_id;
DEFINE INDEX IF NOT EXISTS knowledge_entity_entity_type_idx ON knowledge_entity FIELDS entity_type;
DEFINE INDEX IF NOT EXISTS knowledge_entity_created_at_idx ON knowledge_entity FIELDS created_at;
