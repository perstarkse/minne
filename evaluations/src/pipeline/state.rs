use state_machines::state_machine;

state_machine! {
    name: EvaluationMachine,
    state: EvaluationState,
    initial: Ready,
    states: [Ready, SlicePrepared, DbReady, CorpusReady, NamespaceReady, QueriesFinished, Summarized, Completed, Failed],
    events {
        prepare_slice { transition: { from: Ready, to: SlicePrepared } }
        prepare_db { transition: { from: SlicePrepared, to: DbReady } }
        prepare_corpus { transition: { from: DbReady, to: CorpusReady } }
        prepare_namespace { transition: { from: CorpusReady, to: NamespaceReady } }
        run_queries { transition: { from: NamespaceReady, to: QueriesFinished } }
        summarize { transition: { from: QueriesFinished, to: Summarized } }
        finalize { transition: { from: Summarized, to: Completed } }
        abort {
            transition: { from: Ready, to: Failed }
            transition: { from: SlicePrepared, to: Failed }
            transition: { from: DbReady, to: Failed }
            transition: { from: CorpusReady, to: Failed }
            transition: { from: NamespaceReady, to: Failed }
            transition: { from: QueriesFinished, to: Failed }
            transition: { from: Summarized, to: Failed }
            transition: { from: Completed, to: Failed }
        }
    }
}

pub fn ready() -> EvaluationMachine<(), Ready> {
    EvaluationMachine::new(())
}
