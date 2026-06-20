mod connect;
mod lifecycle;

pub(crate) use connect::{
    can_reuse_namespace, connect_eval_db, default_database, default_namespace, ensure_eval_user,
    namespace_has_corpus, record_namespace_seed, sanitize_model_code,
};
pub(crate) use lifecycle::warm_hnsw_cache;
pub use lifecycle::{recreate_indexes, reset_namespace};
