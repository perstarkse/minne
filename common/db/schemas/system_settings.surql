# Defines the schema for the 'system_settings' table.

DEFINE TABLE IF NOT EXISTS system_settings SCHEMALESS;

# Custom fields from the SystemSettings struct
DEFINE FIELD IF NOT EXISTS registrations_enabled ON system_settings TYPE bool;
DEFINE FIELD IF NOT EXISTS require_email_verification ON system_settings TYPE bool;
DEFINE FIELD IF NOT EXISTS query_model ON system_settings TYPE string;
DEFINE FIELD IF NOT EXISTS processing_model ON system_settings TYPE string;
DEFINE FIELD IF NOT EXISTS image_processing_model ON system_settings TYPE string;
DEFINE FIELD IF NOT EXISTS embedding_model ON system_settings TYPE string;
DEFINE FIELD IF NOT EXISTS embedding_dimensions ON system_settings TYPE int;
DEFINE FIELD IF NOT EXISTS query_system_prompt ON system_settings TYPE string;
DEFINE FIELD IF NOT EXISTS ingestion_system_prompt ON system_settings TYPE string;
DEFINE FIELD IF NOT EXISTS image_processing_prompt ON system_settings TYPE string;
DEFINE FIELD IF NOT EXISTS voice_processing_model ON system_settings TYPE string;
