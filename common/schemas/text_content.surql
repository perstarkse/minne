# Defines the schema for the 'text_content' table.

DEFINE TABLE IF NOT EXISTS text_content SCHEMALESS;

# Standard fields
DEFINE FIELD IF NOT EXISTS created_at ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON text_content TYPE string;

# Custom fields from the TextContent struct
DEFINE FIELD IF NOT EXISTS text ON text_content TYPE string;
# FileInfo is a struct, store as object
DEFINE FIELD IF NOT EXISTS file_info ON text_content TYPE option<object>;
# UrlInfo is a struct, store as object
DEFINE FIELD IF NOT EXISTS url_info ON text_content TYPE option<object>;
DEFINE FIELD IF NOT EXISTS instructions ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS category ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS user_id ON text_content TYPE string;

# Indexes based on query patterns (get_latest_text_contents, get_text_contents_by_category)
DEFINE INDEX IF NOT EXISTS text_content_user_id_idx ON text_content FIELDS user_id;
DEFINE INDEX IF NOT EXISTS text_content_created_at_idx ON text_content FIELDS created_at;
DEFINE INDEX IF NOT EXISTS text_content_category_idx ON text_content FIELDS category;
