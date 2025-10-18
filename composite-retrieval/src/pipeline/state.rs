use state_machines::state_machine;

state_machine! {
    name: HybridRetrievalMachine,
    state: HybridRetrievalState,
    initial: Ready,
    states: [Ready, Embedded, CandidatesLoaded, GraphExpanded, ChunksAttached, Completed, Failed],
    events {
        embed { transition: { from: Ready, to: Embedded } }
        collect_candidates { transition: { from: Embedded, to: CandidatesLoaded } }
        expand_graph { transition: { from: CandidatesLoaded, to: GraphExpanded } }
        attach_chunks { transition: { from: GraphExpanded, to: ChunksAttached } }
        assemble { transition: { from: ChunksAttached, to: Completed } }
        abort {
            transition: { from: Ready, to: Failed }
            transition: { from: CandidatesLoaded, to: Failed }
            transition: { from: GraphExpanded, to: Failed }
            transition: { from: ChunksAttached, to: Failed }
        }
    }
}

pub fn ready() -> HybridRetrievalMachine<(), Ready> {
    HybridRetrievalMachine::new(())
}
