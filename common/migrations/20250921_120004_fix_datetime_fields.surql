-- Align timestamp fields with SurrealDB native datetime type.

-- User timestamps
DEFINE FIELD OVERWRITE created_at ON user FLEXIBLE;
DEFINE FIELD OVERWRITE updated_at ON user FLEXIBLE;

UPDATE user SET created_at = type::datetime(created_at)
WHERE type::is::string(created_at) AND created_at != "";

UPDATE user SET updated_at = type::datetime(updated_at)
WHERE type::is::string(updated_at) AND updated_at != "";

DEFINE FIELD OVERWRITE created_at ON user TYPE datetime;
DEFINE FIELD OVERWRITE updated_at ON user TYPE datetime;

-- Text content timestamps
DEFINE FIELD OVERWRITE created_at ON text_content FLEXIBLE;
DEFINE FIELD OVERWRITE updated_at ON text_content FLEXIBLE;

UPDATE text_content SET created_at = type::datetime(created_at)
WHERE type::is::string(created_at) AND created_at != "";

UPDATE text_content SET updated_at = type::datetime(updated_at)
WHERE type::is::string(updated_at) AND updated_at != "";

DEFINE FIELD OVERWRITE created_at ON text_content TYPE datetime;
DEFINE FIELD OVERWRITE updated_at ON text_content TYPE datetime;

REBUILD INDEX text_content_created_at_idx ON text_content;

-- Text chunk timestamps
DEFINE FIELD OVERWRITE created_at ON text_chunk FLEXIBLE;
DEFINE FIELD OVERWRITE updated_at ON text_chunk FLEXIBLE;

UPDATE text_chunk SET created_at = type::datetime(created_at)
WHERE type::is::string(created_at) AND created_at != "";

UPDATE text_chunk SET updated_at = type::datetime(updated_at)
WHERE type::is::string(updated_at) AND updated_at != "";

DEFINE FIELD OVERWRITE created_at ON text_chunk TYPE datetime;
DEFINE FIELD OVERWRITE updated_at ON text_chunk TYPE datetime;

-- Knowledge entity timestamps
DEFINE FIELD OVERWRITE created_at ON knowledge_entity FLEXIBLE;
DEFINE FIELD OVERWRITE updated_at ON knowledge_entity FLEXIBLE;

UPDATE knowledge_entity SET created_at = type::datetime(created_at)
WHERE type::is::string(created_at) AND created_at != "";

UPDATE knowledge_entity SET updated_at = type::datetime(updated_at)
WHERE type::is::string(updated_at) AND updated_at != "";

DEFINE FIELD OVERWRITE created_at ON knowledge_entity TYPE datetime;
DEFINE FIELD OVERWRITE updated_at ON knowledge_entity TYPE datetime;

REBUILD INDEX knowledge_entity_created_at_idx ON knowledge_entity;

-- Conversation timestamps
DEFINE FIELD OVERWRITE created_at ON conversation FLEXIBLE;
DEFINE FIELD OVERWRITE updated_at ON conversation FLEXIBLE;

UPDATE conversation SET created_at = type::datetime(created_at)
WHERE type::is::string(created_at) AND created_at != "";

UPDATE conversation SET updated_at = type::datetime(updated_at)
WHERE type::is::string(updated_at) AND updated_at != "";

DEFINE FIELD OVERWRITE created_at ON conversation TYPE datetime;
DEFINE FIELD OVERWRITE updated_at ON conversation TYPE datetime;

REBUILD INDEX conversation_created_at_idx ON conversation;

-- Message timestamps
DEFINE FIELD OVERWRITE created_at ON message FLEXIBLE;
DEFINE FIELD OVERWRITE updated_at ON message FLEXIBLE;

UPDATE message SET created_at = type::datetime(created_at)
WHERE type::is::string(created_at) AND created_at != "";

UPDATE message SET updated_at = type::datetime(updated_at)
WHERE type::is::string(updated_at) AND updated_at != "";

DEFINE FIELD OVERWRITE created_at ON message TYPE datetime;
DEFINE FIELD OVERWRITE updated_at ON message TYPE datetime;

REBUILD INDEX message_updated_at_idx ON message;

-- Ingestion task timestamps
DEFINE FIELD OVERWRITE created_at ON ingestion_task FLEXIBLE;
DEFINE FIELD OVERWRITE updated_at ON ingestion_task FLEXIBLE;

UPDATE ingestion_task SET created_at = type::datetime(created_at)
WHERE type::is::string(created_at) AND created_at != "";

UPDATE ingestion_task SET updated_at = type::datetime(updated_at)
WHERE type::is::string(updated_at) AND updated_at != "";

DEFINE FIELD OVERWRITE created_at ON ingestion_task TYPE datetime;
DEFINE FIELD OVERWRITE updated_at ON ingestion_task TYPE datetime;

REBUILD INDEX idx_ingestion_task_created ON ingestion_task;

-- File timestamps
DEFINE FIELD OVERWRITE created_at ON file FLEXIBLE;
DEFINE FIELD OVERWRITE updated_at ON file FLEXIBLE;

UPDATE file SET created_at = type::datetime(created_at)
WHERE type::is::string(created_at) AND created_at != "";

UPDATE file SET updated_at = type::datetime(updated_at)
WHERE type::is::string(updated_at) AND updated_at != "";

DEFINE FIELD OVERWRITE created_at ON file TYPE datetime;
DEFINE FIELD OVERWRITE updated_at ON file TYPE datetime;
