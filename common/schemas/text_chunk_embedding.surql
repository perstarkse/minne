-- Defines the schema for the 'text_chunk_embedding' table.
-- Separate table to optimize HNSW index creation memory usage

DEFINE TABLE IF NOT EXISTS text_chunk_embedding SCHEMAFULL;

# Standard fields
DEFINE FIELD IF NOT EXISTS created_at ON text_chunk_embedding TYPE datetime;
DEFINE FIELD IF NOT EXISTS updated_at ON text_chunk_embedding TYPE datetime;
DEFINE FIELD IF NOT EXISTS user_id ON text_chunk_embedding TYPE string;
DEFINE FIELD IF NOT EXISTS source_id ON text_chunk_embedding TYPE string;

# Custom fields
DEFINE FIELD IF NOT EXISTS chunk_id ON text_chunk_embedding TYPE record<text_chunk>;
DEFINE FIELD IF NOT EXISTS embedding ON text_chunk_embedding TYPE array<float>;

-- Indexes
-- DEFINE INDEX IF NOT EXISTS idx_embedding_text_chunk_embedding ON text_chunk_embedding FIELDS embedding HNSW DIMENSION 1536;
DEFINE INDEX IF NOT EXISTS text_chunk_embedding_chunk_id_idx ON text_chunk_embedding FIELDS chunk_id;
DEFINE INDEX IF NOT EXISTS text_chunk_embedding_user_id_idx ON text_chunk_embedding FIELDS user_id;
DEFINE INDEX IF NOT EXISTS text_chunk_embedding_source_id_idx ON text_chunk_embedding FIELDS source_id;
