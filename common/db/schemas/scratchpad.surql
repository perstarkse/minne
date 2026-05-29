# Defines the schema for the 'scratchpad' table.

DEFINE TABLE IF NOT EXISTS scratchpad SCHEMALESS;

# Standard fields from stored_object! macro
DEFINE FIELD IF NOT EXISTS created_at ON scratchpad TYPE datetime;
DEFINE FIELD IF NOT EXISTS updated_at ON scratchpad TYPE datetime;

# Custom fields from the Scratchpad struct
DEFINE FIELD IF NOT EXISTS user_id ON scratchpad TYPE string;
DEFINE FIELD IF NOT EXISTS title ON scratchpad TYPE string;
DEFINE FIELD IF NOT EXISTS content ON scratchpad TYPE string;
DEFINE FIELD IF NOT EXISTS last_saved_at ON scratchpad TYPE datetime;
DEFINE FIELD IF NOT EXISTS is_dirty ON scratchpad TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS is_archived ON scratchpad TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS archived_at ON scratchpad TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS ingested_at ON scratchpad TYPE option<datetime>;

# Indexes based on query patterns
DEFINE INDEX IF NOT EXISTS scratchpad_user_idx ON scratchpad FIELDS user_id;
DEFINE INDEX IF NOT EXISTS scratchpad_user_archived_idx ON scratchpad FIELDS user_id, is_archived;
DEFINE INDEX IF NOT EXISTS scratchpad_updated_idx ON scratchpad FIELDS updated_at;
DEFINE INDEX IF NOT EXISTS scratchpad_archived_idx ON scratchpad FIELDS archived_at;
