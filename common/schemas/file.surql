# Defines the schema for the 'file' table (used by FileInfo).

DEFINE TABLE IF NOT EXISTS file SCHEMALESS;

# Standard fields
DEFINE FIELD IF NOT EXISTS created_at ON file TYPE datetime;
DEFINE FIELD IF NOT EXISTS updated_at ON file TYPE datetime;

# Custom fields from the FileInfo struct
DEFINE FIELD IF NOT EXISTS sha256 ON file TYPE string;
DEFINE FIELD IF NOT EXISTS path ON file TYPE string;
DEFINE FIELD IF NOT EXISTS file_name ON file TYPE string;
DEFINE FIELD IF NOT EXISTS mime_type ON file TYPE string;
DEFINE FIELD IF NOT EXISTS user_id ON file TYPE string;

# Indexes based on usage (get_by_sha scoped by user_id, user lookups)
DEFINE INDEX IF NOT EXISTS file_user_sha256_idx ON file FIELDS user_id, sha256 UNIQUE;
DEFINE INDEX IF NOT EXISTS file_user_id_idx ON file FIELDS user_id;
