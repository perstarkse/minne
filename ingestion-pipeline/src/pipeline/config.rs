#[derive(Debug, Clone)]
pub struct IngestionTuning {
    pub retry_base_delay_secs: u64,
    pub retry_max_delay_secs: u64,
    pub retry_backoff_cap_exponent: u32,
    pub persist_attempts: usize,
    pub persist_initial_backoff_ms: u64,
    pub persist_max_backoff_ms: u64,
    pub chunk_min_tokens: usize,
    pub chunk_max_tokens: usize,
    pub chunk_overlap_tokens: usize,
    pub entity_embedding_concurrency: usize,
    /// Maximum characters of content body used to build the similarity-search query
    /// during retrieval. Longer bodies are truncated to keep embedding inputs bounded.
    pub embedding_query_char_limit: usize,
}

impl Default for IngestionTuning {
    fn default() -> Self {
        Self {
            retry_base_delay_secs: 30,
            retry_max_delay_secs: 15 * 60,
            retry_backoff_cap_exponent: 5,
            persist_attempts: 3,
            persist_initial_backoff_ms: 50,
            persist_max_backoff_ms: 800,
            chunk_min_tokens: 256,
            chunk_max_tokens: 512,
            chunk_overlap_tokens: 50,
            entity_embedding_concurrency: 4,
            embedding_query_char_limit: 12_000,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct IngestionConfig {
    pub tuning: IngestionTuning,
    pub chunk_only: bool,
}
