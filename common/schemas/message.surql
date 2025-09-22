# Defines the schema for the 'message' table.

DEFINE TABLE IF NOT EXISTS message SCHEMALESS;

# Standard fields
DEFINE FIELD IF NOT EXISTS created_at ON message TYPE datetime;
DEFINE FIELD IF NOT EXISTS updated_at ON message TYPE datetime;

# Custom fields from the Message struct
DEFINE FIELD IF NOT EXISTS conversation_id ON message TYPE string;
# MessageRole is an enum, store as string
DEFINE FIELD IF NOT EXISTS role ON message TYPE string;
DEFINE FIELD IF NOT EXISTS content ON message TYPE string;
# references is Option<Vec<String>>, store as array<string>
DEFINE FIELD IF NOT EXISTS references ON message TYPE option<array<string>>;

# Indexes based on query patterns (get_complete_conversation)
DEFINE INDEX IF NOT EXISTS message_conversation_id_idx ON message FIELDS conversation_id;
DEFINE INDEX IF NOT EXISTS message_updated_at_idx ON message FIELDS updated_at; # For ORDER BY
