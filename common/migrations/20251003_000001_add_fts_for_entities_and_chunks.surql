-- Add FTS indexes for searching name and description on entities

DEFINE ANALYZER IF NOT EXISTS app_en_fts_analyzer
    TOKENIZERS class
    FILTERS lowercase, ascii, snowball(english);

DEFINE INDEX IF NOT EXISTS knowledge_entity_fts_name_idx ON TABLE knowledge_entity
    FIELDS name
    SEARCH ANALYZER app_en_fts_analyzer BM25;

DEFINE INDEX IF NOT EXISTS knowledge_entity_fts_description_idx ON TABLE knowledge_entity
    FIELDS description
    SEARCH ANALYZER app_en_fts_analyzer BM25;

DEFINE INDEX IF NOT EXISTS text_chunk_fts_chunk_idx ON TABLE text_chunk
    FIELDS chunk
    SEARCH ANALYZER app_en_fts_analyzer BM25;
