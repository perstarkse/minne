# Defines the schema for the 'user' table.
# NOTE: Authentication scope and access rules are defined in auth.surql

DEFINE TABLE IF NOT EXISTS user SCHEMALESS;

# Standard fields
DEFINE FIELD IF NOT EXISTS created_at ON user TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON user TYPE string;

# Custom fields from the User struct
DEFINE FIELD IF NOT EXISTS email ON user TYPE string;
DEFINE FIELD IF NOT EXISTS password ON user TYPE string; # Stores the hashed password
DEFINE FIELD IF NOT EXISTS anonymous ON user TYPE bool;
DEFINE FIELD IF NOT EXISTS api_key ON user TYPE option<string>;
DEFINE FIELD IF NOT EXISTS admin ON user TYPE bool;
DEFINE FIELD IF NOT EXISTS timezone ON user TYPE string;

# Indexes based on query patterns (find_by_email, find_by_api_key, unique constraint from setup_auth)
DEFINE INDEX IF NOT EXISTS user_email_idx ON user FIELDS email UNIQUE;
DEFINE INDEX IF NOT EXISTS user_api_key_idx ON user FIELDS api_key;
