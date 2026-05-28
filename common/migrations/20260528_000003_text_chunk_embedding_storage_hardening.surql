-- Harden text chunk embeddings storage invariants.

-- Re-key embeddings so record id matches chunk id (stable 1:1 identity).
FOR $emb IN (SELECT * FROM text_chunk_embedding) {
    LET $chunk_key = record::id($emb.chunk_id);
    LET $canonical = type::thing('text_chunk_embedding', $chunk_key);
    IF $emb.id != $canonical {
        UPSERT $canonical CONTENT {
            chunk_id: $emb.chunk_id,
            embedding: $emb.embedding,
            user_id: $emb.user_id,
            source_id: $emb.source_id,
            created_at: $emb.created_at,
            updated_at: $emb.updated_at
        };
        DELETE $emb.id;
    }
};

REMOVE INDEX IF EXISTS text_chunk_embedding_chunk_id_idx ON text_chunk_embedding;
DEFINE INDEX IF NOT EXISTS text_chunk_embedding_chunk_id_idx ON text_chunk_embedding FIELDS chunk_id UNIQUE;
