-- Add embedding_backend field to system_settings for visibility of active backend

DEFINE FIELD IF NOT EXISTS embedding_backend ON system_settings TYPE option<string>;

-- Set default to 'openai' for existing installs to preserve backward compatibility
UPDATE system_settings:current SET
    embedding_backend = 'openai'
WHERE embedding_backend == NONE;
