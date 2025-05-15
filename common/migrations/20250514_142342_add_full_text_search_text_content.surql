DEFINE ANALYZER IF NOT EXISTS app_default_fts_analyzer
    TOKENIZERS class
    FILTERS lowercase, ascii; 

DEFINE INDEX IF NOT EXISTS text_content_fts_text_idx ON TABLE text_content
    FIELDS text
    SEARCH ANALYZER app_default_fts_analyzer BM25 HIGHLIGHTS;

DEFINE INDEX IF NOT EXISTS text_content_fts_category_idx ON TABLE text_content
    FIELDS category
    SEARCH ANALYZER app_default_fts_analyzer BM25 HIGHLIGHTS;

DEFINE INDEX IF NOT EXISTS text_content_fts_context_idx ON TABLE text_content
    FIELDS context
    SEARCH ANALYZER app_default_fts_analyzer BM25 HIGHLIGHTS;

DEFINE INDEX IF NOT EXISTS text_content_fts_file_name_idx ON TABLE text_content
    FIELDS file_info.file_name
    SEARCH ANALYZER app_default_fts_analyzer BM25 HIGHLIGHTS;

DEFINE INDEX IF NOT EXISTS text_content_fts_url_idx ON TABLE text_content
    FIELDS url_info.url
    SEARCH ANALYZER app_default_fts_analyzer BM25 HIGHLIGHTS;

DEFINE INDEX IF NOT EXISTS text_content_fts_url_title_idx ON TABLE text_content
    FIELDS url_info.title
    SEARCH ANALYZER app_default_fts_analyzer BM25 HIGHLIGHTS;
