# Defines the schema for the 'knowledge_entity' table.

DEFINE TABLE IF NOT EXISTS knowledge_entity SCHEMALESS;

# Standard fields
DEFINE FIELD IF NOT EXISTS created_at ON knowledge_entity TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON knowledge_entity TYPE string;

# Custom fields from the KnowledgeEntity struct
DEFINE FIELD IF NOT EXISTS source_id ON knowledge_entity TYPE string;
DEFINE FIELD IF NOT EXISTS name ON knowledge_entity TYPE string;
DEFINE FIELD IF NOT EXISTS description ON knowledge_entity TYPE string;
# KnowledgeEntityType is an enum, store as string
DEFINE FIELD IF NOT EXISTS entity_type ON knowledge_entity TYPE string;
# metadata is Option<serde_json::Value>, store as object
DEFINE FIELD IF NOT EXISTS metadata ON knowledge_entity TYPE option<object>;

# Define embedding as a standard array of floats for schema definition
DEFINE FIELD IF NOT EXISTS embedding ON knowledge_entity TYPE array<float>;
# The specific vector nature is handled by the index definition below

DEFINE FIELD IF NOT EXISTS user_id ON knowledge_entity TYPE string;

# Indexes based on build_indexes and query patterns
# The INDEX definition correctly specifies the vector properties
DEFINE INDEX IF NOT EXISTS idx_embedding_entities ON knowledge_entity FIELDS embedding HNSW DIMENSION 1536;
DEFINE INDEX IF NOT EXISTS knowledge_entity_user_id_idx ON knowledge_entity FIELDS user_id;
DEFINE INDEX IF NOT EXISTS knowledge_entity_source_id_idx ON knowledge_entity FIELDS source_id;
DEFINE INDEX IF NOT EXISTS knowledge_entity_entity_type_idx ON knowledge_entity FIELDS entity_type;
DEFINE INDEX IF NOT EXISTS knowledge_entity_created_at_idx ON knowledge_entity FIELDS created_at; # For get_latest_knowledge_entities
