# Defines the schema for the 'text_content' table.

DEFINE TABLE IF NOT EXISTS text_content SCHEMAFULL;

# Standard fields
DEFINE FIELD IF NOT EXISTS created_at ON text_content TYPE datetime;
DEFINE FIELD IF NOT EXISTS updated_at ON text_content TYPE datetime;

# Custom fields from the TextContent struct
DEFINE FIELD IF NOT EXISTS text ON text_content TYPE string;
# FileInfo is a struct, store as object
DEFINE FIELD IF NOT EXISTS file_info ON text_content TYPE option<object>;
# UrlInfo is a struct, store as object
DEFINE FIELD IF NOT EXISTS url_info ON text_content TYPE option<object>;
DEFINE FIELD IF NOT EXISTS url_info.url ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS url_info.title ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS url_info.image_id ON text_content TYPE string;

DEFINE FIELD IF NOT EXISTS context ON text_content TYPE option<string>;
DEFINE FIELD IF NOT EXISTS category ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS user_id ON text_content TYPE string;

# FileInfo fields
DEFINE FIELD IF NOT EXISTS file_info.id ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS file_info.created_at ON text_content TYPE datetime;
DEFINE FIELD IF NOT EXISTS file_info.updated_at ON text_content TYPE datetime;
DEFINE FIELD IF NOT EXISTS file_info.sha256 ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS file_info.path ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS file_info.file_name ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS file_info.mime_type ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS file_info.user_id ON text_content TYPE string;

# Indexes based on query patterns
DEFINE INDEX IF NOT EXISTS text_content_user_id_idx ON text_content FIELDS user_id;
DEFINE INDEX IF NOT EXISTS text_content_created_at_idx ON text_content FIELDS created_at;
DEFINE INDEX IF NOT EXISTS text_content_category_idx ON text_content FIELDS category;
