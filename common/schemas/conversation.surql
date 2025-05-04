# Defines the schema for the 'conversation' table.

DEFINE TABLE IF NOT EXISTS conversation SCHEMALESS;

# Standard fields
DEFINE FIELD IF NOT EXISTS created_at ON conversation TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON conversation TYPE string;

# Custom fields from the Conversation struct
DEFINE FIELD IF NOT EXISTS user_id ON conversation TYPE string;
DEFINE FIELD IF NOT EXISTS title ON conversation TYPE string;

# Add indexes based on query patterns (get_complete_conversation ownership check, get_user_conversations)
DEFINE INDEX IF NOT EXISTS conversation_user_id_idx ON conversation FIELDS user_id;
DEFINE INDEX IF NOT EXISTS conversation_created_at_idx ON conversation FIELDS created_at; # For get_user_conversations ORDER BY
