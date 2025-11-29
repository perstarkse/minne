#[derive(Debug, Clone)]
pub struct IngestionTuning {
    pub retry_base_delay_secs: u64,
    pub retry_max_delay_secs: u64,
    pub retry_backoff_cap_exponent: u32,
    pub graph_store_attempts: usize,
    pub graph_initial_backoff_ms: u64,
    pub graph_max_backoff_ms: u64,
    pub chunk_min_tokens: usize,
    pub chunk_max_tokens: usize,
    pub chunk_insert_concurrency: usize,
    pub entity_embedding_concurrency: usize,
}

impl Default for IngestionTuning {
    fn default() -> Self {
        Self {
            retry_base_delay_secs: 30,
            retry_max_delay_secs: 15 * 60,
            retry_backoff_cap_exponent: 5,
            graph_store_attempts: 3,
            graph_initial_backoff_ms: 50,
            graph_max_backoff_ms: 800,
            chunk_min_tokens: 500,
            chunk_max_tokens: 2_000,
            chunk_insert_concurrency: 8,
            entity_embedding_concurrency: 4,
        }
    }
}

#[derive(Debug, Clone)]
pub struct IngestionConfig {
    pub tuning: IngestionTuning,
    pub chunk_only: bool,
}

impl Default for IngestionConfig {
    fn default() -> Self {
        Self {
            tuning: IngestionTuning::default(),
            chunk_only: false,
        }
    }
}
