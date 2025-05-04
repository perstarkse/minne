# Defines the 'relates_to' edge table for KnowledgeRelationships.
# Edges connect nodes, in this case knowledge_entity records.

# Define the edge table itself, enforcing connections between knowledge_entity records
# SCHEMAFULL requires all fields to be defined, maybe start with SCHEMALESS if metadata might vary
DEFINE TABLE IF NOT EXISTS relates_to SCHEMALESS TYPE RELATION FROM knowledge_entity TO knowledge_entity;

# Define the metadata field within the edge
# RelationshipMetadata is a struct, store as object
DEFINE FIELD IF NOT EXISTS metadata ON relates_to TYPE object;

# Optionally, define fields within the metadata object for stricter schema (requires SCHEMAFULL on table)
# DEFINE FIELD IF NOT EXISTS metadata.user_id ON relates_to TYPE string;
# DEFINE FIELD IF NOT EXISTS metadata.source_id ON relates_to TYPE string;
# DEFINE FIELD IF NOT EXISTS metadata.relationship_type ON relates_to TYPE string;

# Add indexes based on query patterns (delete_relationships_by_source_id, get_knowledge_relationships)
DEFINE INDEX IF NOT EXISTS relates_to_metadata_source_id_idx ON relates_to FIELDS metadata.source_id;
DEFINE INDEX IF NOT EXISTS relates_to_metadata_user_id_idx ON relates_to FIELDS metadata.user_id;
