# Defines the schema for the 'text_chunk' table.

DEFINE TABLE IF NOT EXISTS text_chunk SCHEMALESS;

# Standard fields
DEFINE FIELD IF NOT EXISTS created_at ON text_chunk TYPE datetime;
DEFINE FIELD IF NOT EXISTS updated_at ON text_chunk TYPE datetime;

# Custom fields from the TextChunk struct
DEFINE FIELD IF NOT EXISTS source_id ON text_chunk TYPE string;
DEFINE FIELD IF NOT EXISTS chunk ON text_chunk TYPE string;

DEFINE FIELD IF NOT EXISTS user_id ON text_chunk TYPE string;

# Indexes based on build_indexes and query patterns (delete_by_source_id)
DEFINE INDEX IF NOT EXISTS text_chunk_source_id_idx ON text_chunk FIELDS source_id;
DEFINE INDEX IF NOT EXISTS text_chunk_user_id_idx ON text_chunk FIELDS user_id;
