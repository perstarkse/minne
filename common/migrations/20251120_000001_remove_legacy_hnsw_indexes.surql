-- Remove HNSW indexes from base tables (now created at runtime on *_embedding tables)
REMOVE INDEX IF EXISTS idx_embedding_entities ON knowledge_entity;
REMOVE INDEX IF EXISTS idx_embedding_chunks ON text_chunk;

-- Remove FTS indexes (now created at runtime via indexes.rs)
REMOVE INDEX IF EXISTS text_content_fts_text_idx ON text_content;
REMOVE INDEX IF EXISTS text_content_fts_category_idx ON text_content;
REMOVE INDEX IF EXISTS text_content_fts_context_idx ON text_content;
REMOVE INDEX IF EXISTS text_content_fts_file_name_idx ON text_content;
REMOVE INDEX IF EXISTS text_content_fts_url_idx ON text_content;
REMOVE INDEX IF EXISTS text_content_fts_url_title_idx ON text_content;
REMOVE INDEX IF EXISTS knowledge_entity_fts_name_idx ON knowledge_entity;
REMOVE INDEX IF EXISTS knowledge_entity_fts_description_idx ON knowledge_entity;
REMOVE INDEX IF EXISTS text_chunk_fts_chunk_idx ON text_chunk;

-- Remove legacy analyzers (recreated at runtime with updated configuration)
REMOVE ANALYZER IF EXISTS app_default_fts_analyzer;
REMOVE ANALYZER IF EXISTS app_en_fts_analyzer;
