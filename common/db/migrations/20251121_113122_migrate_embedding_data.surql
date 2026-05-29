-- Copy embeddings from base tables to dedicated tables
-- This runs BEFORE the field removal migration

FOR $chunk IN (SELECT * FROM text_chunk WHERE embedding != NONE AND array::len(embedding) > 0) {
    CREATE text_chunk_embedding CONTENT {
        chunk_id: $chunk.id,
        embedding: $chunk.embedding,
        user_id: $chunk.user_id,
        source_id: $chunk.source_id,
        created_at: $chunk.created_at,
        updated_at: $chunk.updated_at
    };
};

FOR $entity IN (SELECT * FROM knowledge_entity WHERE embedding != NONE AND array::len(embedding) > 0) {
    CREATE knowledge_entity_embedding CONTENT {
        entity_id: $entity.id,
        embedding: $entity.embedding,
        user_id: $entity.user_id,
        created_at: $entity.created_at,
        updated_at: $entity.updated_at
    };
};
