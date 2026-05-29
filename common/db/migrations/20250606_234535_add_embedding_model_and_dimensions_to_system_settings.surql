DEFINE FIELD IF NOT EXISTS embedding_model ON system_settings TYPE string;
DEFINE FIELD IF NOT EXISTS embedding_dimensions ON system_settings TYPE int;

UPDATE system_settings:current SET
	embedding_model =  "text-embedding-3-small",
    embedding_dimensions = 1536
WHERE embedding_model == NONE && embedding_dimensions == NONE;
