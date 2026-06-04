-- Align persisted embedding settings when FastEmbed is the recorded backend but the model
-- name is still the OpenAI migration default (invalid for FastEmbed `from_str`).

UPDATE system_settings:current SET
    embedding_model = 'Xenova/bge-small-en-v1.5',
    embedding_dimensions = 384
WHERE embedding_backend = 'fastembed'
    AND embedding_model = 'text-embedding-3-small';
