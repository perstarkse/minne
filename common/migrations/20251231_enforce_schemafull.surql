-- Enforce SCHEMAFULL on all tables and define missing fields

-- 1. Define missing fields for ingestion_task (formerly job, but now ingestion_task)
DEFINE TABLE OVERWRITE ingestion_task SCHEMAFULL;

-- Core Fields
DEFINE FIELD IF NOT EXISTS id ON ingestion_task TYPE record<ingestion_task>;
DEFINE FIELD IF NOT EXISTS created_at ON ingestion_task TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS updated_at ON ingestion_task TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS user_id ON ingestion_task TYPE string;

-- State Machine Fields
DEFINE FIELD IF NOT EXISTS state ON ingestion_task TYPE string ASSERT $value IN ['Pending', 'Reserved', 'Processing', 'Succeeded', 'Failed', 'Cancelled', 'DeadLetter'];
DEFINE FIELD IF NOT EXISTS attempts ON ingestion_task TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS max_attempts ON ingestion_task TYPE int DEFAULT 3;
DEFINE FIELD IF NOT EXISTS scheduled_at ON ingestion_task TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS locked_at ON ingestion_task TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS lease_duration_secs ON ingestion_task TYPE int DEFAULT 300;
DEFINE FIELD IF NOT EXISTS worker_id ON ingestion_task TYPE option<string>;
DEFINE FIELD IF NOT EXISTS error_code ON ingestion_task TYPE option<string>;
DEFINE FIELD IF NOT EXISTS error_message ON ingestion_task TYPE option<string>;
DEFINE FIELD IF NOT EXISTS last_error_at ON ingestion_task TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS priority ON ingestion_task TYPE int DEFAULT 0;

-- Content Payload (IngestionPayload Enum)
DEFINE FIELD IF NOT EXISTS content ON ingestion_task TYPE object;
DEFINE FIELD IF NOT EXISTS content.Url ON ingestion_task TYPE option<object>;
DEFINE FIELD IF NOT EXISTS content.Text ON ingestion_task TYPE option<object>;
DEFINE FIELD IF NOT EXISTS content.File ON ingestion_task TYPE option<object>;

-- Content: Url Variant
DEFINE FIELD IF NOT EXISTS content.Url.url ON ingestion_task TYPE string;
DEFINE FIELD IF NOT EXISTS content.Url.context ON ingestion_task TYPE string;
DEFINE FIELD IF NOT EXISTS content.Url.category ON ingestion_task TYPE string;
DEFINE FIELD IF NOT EXISTS content.Url.user_id ON ingestion_task TYPE string;

-- Content: Text Variant
DEFINE FIELD IF NOT EXISTS content.Text.text ON ingestion_task TYPE string;
DEFINE FIELD IF NOT EXISTS content.Text.context ON ingestion_task TYPE string;
DEFINE FIELD IF NOT EXISTS content.Text.category ON ingestion_task TYPE string;
DEFINE FIELD IF NOT EXISTS content.Text.user_id ON ingestion_task TYPE string;

-- Content: File Variant
DEFINE FIELD IF NOT EXISTS content.File.context ON ingestion_task TYPE string;
DEFINE FIELD IF NOT EXISTS content.File.category ON ingestion_task TYPE string;
DEFINE FIELD IF NOT EXISTS content.File.user_id ON ingestion_task TYPE string;
DEFINE FIELD IF NOT EXISTS content.File.file_info ON ingestion_task TYPE object;

-- Content: File.file_info (FileInfo Struct)
DEFINE FIELD IF NOT EXISTS content.File.file_info.id ON ingestion_task TYPE string;
DEFINE FIELD IF NOT EXISTS content.File.file_info.created_at ON ingestion_task TYPE datetime;
DEFINE FIELD IF NOT EXISTS content.File.file_info.updated_at ON ingestion_task TYPE datetime;
DEFINE FIELD IF NOT EXISTS content.File.file_info.sha256 ON ingestion_task TYPE string;
DEFINE FIELD IF NOT EXISTS content.File.file_info.path ON ingestion_task TYPE string;
DEFINE FIELD IF NOT EXISTS content.File.file_info.file_name ON ingestion_task TYPE string;
DEFINE FIELD IF NOT EXISTS content.File.file_info.mime_type ON ingestion_task TYPE string;
DEFINE FIELD IF NOT EXISTS content.File.file_info.user_id ON ingestion_task TYPE string;

-- 2. Enforce SCHEMAFULL on all other tables
DEFINE TABLE OVERWRITE analytics SCHEMAFULL;
DEFINE TABLE OVERWRITE conversation SCHEMAFULL;
DEFINE TABLE OVERWRITE file SCHEMAFULL;
DEFINE TABLE OVERWRITE knowledge_entity SCHEMAFULL;
DEFINE TABLE OVERWRITE message SCHEMAFULL;
DEFINE TABLE OVERWRITE relates_to SCHEMAFULL;
DEFINE TABLE OVERWRITE scratchpad SCHEMAFULL;
DEFINE TABLE OVERWRITE system_settings SCHEMAFULL;
DEFINE TABLE OVERWRITE text_chunk SCHEMAFULL;
-- text_content must have fields defined before enforcing SCHEMAFULL
DEFINE TABLE OVERWRITE text_content SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS created_at ON text_content TYPE datetime;
DEFINE FIELD IF NOT EXISTS updated_at ON text_content TYPE datetime;
DEFINE FIELD IF NOT EXISTS text ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS file_info ON text_content TYPE option<object>;
DEFINE FIELD IF NOT EXISTS url_info ON text_content TYPE option<object>;
DEFINE FIELD IF NOT EXISTS url_info.url ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS url_info.title ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS url_info.image_id ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS context ON text_content TYPE option<string>;
DEFINE FIELD IF NOT EXISTS category ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS user_id ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS file_info.id ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS file_info.created_at ON text_content TYPE datetime;
DEFINE FIELD IF NOT EXISTS file_info.updated_at ON text_content TYPE datetime;
DEFINE FIELD IF NOT EXISTS file_info.sha256 ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS file_info.path ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS file_info.file_name ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS file_info.mime_type ON text_content TYPE string;
DEFINE FIELD IF NOT EXISTS file_info.user_id ON text_content TYPE string;

DEFINE TABLE OVERWRITE user SCHEMAFULL;
