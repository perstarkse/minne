-- Harden knowledge entity embeddings and graph storage invariants.

DEFINE FIELD IF NOT EXISTS source_id ON knowledge_entity_embedding TYPE string;

-- Backfill denormalized source_id from the linked entity.
FOR $emb IN (SELECT * FROM knowledge_entity_embedding WHERE source_id = NONE OR source_id = '') {
    LET $entity = (SELECT source_id FROM $emb.entity_id)[0];
    IF $entity != NONE {
        UPDATE $emb.id SET source_id = $entity.source_id;
    }
};

-- Re-key embeddings so record id matches entity id (stable 1:1 identity).
FOR $emb IN (SELECT * FROM knowledge_entity_embedding) {
    LET $entity_key = record::id($emb.entity_id);
    LET $canonical = type::thing('knowledge_entity_embedding', $entity_key);
    IF $emb.id != $canonical {
        UPSERT $canonical CONTENT {
            entity_id: $emb.entity_id,
            embedding: $emb.embedding,
            user_id: $emb.user_id,
            source_id: $emb.source_id,
            created_at: $emb.created_at,
            updated_at: $emb.updated_at
        };
        DELETE $emb.id;
    }
};

REMOVE INDEX IF EXISTS knowledge_entity_embedding_entity_id_idx ON knowledge_entity_embedding;
DEFINE INDEX IF NOT EXISTS knowledge_entity_embedding_entity_id_idx ON knowledge_entity_embedding FIELDS entity_id UNIQUE;
DEFINE INDEX IF NOT EXISTS knowledge_entity_embedding_source_id_idx ON knowledge_entity_embedding FIELDS source_id;
DEFINE INDEX IF NOT EXISTS knowledge_entity_user_source_idx ON knowledge_entity FIELDS user_id, source_id;
