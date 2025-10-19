use state_machines::state_machine;

state_machine! {
    name: IngestionMachine,
    state: IngestionState,
    initial: Ready,
    states: [Ready, ContentPrepared, Retrieved, Enriched, Persisted, Failed],
    events {
        prepare { transition: { from: Ready, to: ContentPrepared } }
        retrieve { transition: { from: ContentPrepared, to: Retrieved } }
        enrich { transition: { from: Retrieved, to: Enriched } }
        persist { transition: { from: Enriched, to: Persisted } }
        abort {
            transition: { from: Ready, to: Failed }
            transition: { from: ContentPrepared, to: Failed }
            transition: { from: Retrieved, to: Failed }
            transition: { from: Enriched, to: Failed }
            transition: { from: Persisted, to: Failed }
        }
    }
}

pub fn ready() -> IngestionMachine<(), Ready> {
    IngestionMachine::new(())
}
