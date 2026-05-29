-- Drop legacy embedding fields from base tables; embeddings now live in *_embedding tables.
REMOVE FIELD IF EXISTS embedding ON TABLE text_chunk;
REMOVE FIELD IF EXISTS embedding ON TABLE knowledge_entity;
