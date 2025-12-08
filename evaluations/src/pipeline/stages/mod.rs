mod finalize;
mod prepare_corpus;
mod prepare_db;
mod prepare_namespace;
mod prepare_slice;
mod run_queries;
mod summarize;

pub(crate) use finalize::finalize;
pub(crate) use prepare_corpus::prepare_corpus;
pub(crate) use prepare_db::prepare_db;
pub(crate) use prepare_namespace::prepare_namespace;
pub(crate) use prepare_slice::prepare_slice;
pub(crate) use run_queries::run_queries;
pub(crate) use summarize::summarize;

use anyhow::Result;
use state_machines::core::GuardError;

use super::state::EvaluationMachine;

fn map_guard_error(event: &str, guard: GuardError) -> anyhow::Error {
    anyhow::anyhow!("invalid evaluation pipeline transition during {event}: {guard:?}")
}

type StageResult<S> = Result<EvaluationMachine<(), S>>;
