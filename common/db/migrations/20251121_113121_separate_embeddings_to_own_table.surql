-- Move chunk/entity embeddings to dedicated tables for index efficiency.

-- Text chunk embeddings table
DEFINE TABLE IF NOT EXISTS text_chunk_embedding SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS created_at ON text_chunk_embedding TYPE datetime;
DEFINE FIELD IF NOT EXISTS updated_at ON text_chunk_embedding TYPE datetime;
DEFINE FIELD IF NOT EXISTS user_id ON text_chunk_embedding TYPE string;
DEFINE FIELD IF NOT EXISTS source_id ON text_chunk_embedding TYPE string;
DEFINE FIELD IF NOT EXISTS chunk_id ON text_chunk_embedding TYPE record<text_chunk>;
DEFINE FIELD IF NOT EXISTS embedding ON text_chunk_embedding TYPE array<float>;
DEFINE INDEX IF NOT EXISTS text_chunk_embedding_chunk_id_idx ON text_chunk_embedding FIELDS chunk_id;
DEFINE INDEX IF NOT EXISTS text_chunk_embedding_user_id_idx ON text_chunk_embedding FIELDS user_id;
DEFINE INDEX IF NOT EXISTS text_chunk_embedding_source_id_idx ON text_chunk_embedding FIELDS source_id;

-- Knowledge entity embeddings table
DEFINE TABLE IF NOT EXISTS knowledge_entity_embedding SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS created_at ON knowledge_entity_embedding TYPE datetime;
DEFINE FIELD IF NOT EXISTS updated_at ON knowledge_entity_embedding TYPE datetime;
DEFINE FIELD IF NOT EXISTS user_id ON knowledge_entity_embedding TYPE string;
DEFINE FIELD IF NOT EXISTS entity_id ON knowledge_entity_embedding TYPE record<knowledge_entity>;
DEFINE FIELD IF NOT EXISTS embedding ON knowledge_entity_embedding TYPE array<float>;
DEFINE INDEX IF NOT EXISTS knowledge_entity_embedding_entity_id_idx ON knowledge_entity_embedding FIELDS entity_id;
DEFINE INDEX IF NOT EXISTS knowledge_entity_embedding_user_id_idx ON knowledge_entity_embedding FIELDS user_id;
