-- Defines the schema for the 'knowledge_entity_embedding' table.
-- Separate table to optimize HNSW index creation memory usage

DEFINE TABLE IF NOT EXISTS knowledge_entity_embedding SCHEMAFULL;

-- Standard fields
DEFINE FIELD IF NOT EXISTS created_at ON knowledge_entity_embedding TYPE datetime;
DEFINE FIELD IF NOT EXISTS updated_at ON knowledge_entity_embedding TYPE datetime;
DEFINE FIELD IF NOT EXISTS user_id ON knowledge_entity_embedding TYPE string;

-- Custom fields
DEFINE FIELD IF NOT EXISTS entity_id ON knowledge_entity_embedding TYPE record<knowledge_entity>;
DEFINE FIELD IF NOT EXISTS embedding ON knowledge_entity_embedding TYPE array<float>;

-- Indexes
-- DEFINE INDEX IF NOT EXISTS idx_embedding_knowledge_entity_embedding ON knowledge_entity_embedding FIELDS embedding HNSW DIMENSION 1536;
DEFINE INDEX IF NOT EXISTS knowledge_entity_embedding_entity_id_idx ON knowledge_entity_embedding FIELDS entity_id;
DEFINE INDEX IF NOT EXISTS knowledge_entity_embedding_user_id_idx ON knowledge_entity_embedding FIELDS user_id;
