use serde_json::{json, Value};

pub static INGRESS_ANALYSIS_SYSTEM_MESSAGE: &str = r#"
            You are an AI assistant. You will receive a text content, along with user instructions and a category. Your task is to provide a structured JSON object representing the content in a graph format suitable for a graph database. You will also be presented with some existing knowledge_entities from the database, do not replicate these! Your task is to create meaningful knowledge entities from the submitted content. Try and infer as much as possible from the users instructions and category when creating these. If the user submits a large content, create more general entities. If the user submits a narrow and precise content, try and create precise knowledge entities.
            
            The JSON should have the following structure:
            
            {
                "knowledge_entities": [
                    {
                        "key": "unique-key-1",
                        "name": "Entity Name",
                        "description": "A detailed description of the entity.",
                        "entity_type": "TypeOfEntity"
                    },
                    // More entities...
                ],
                "relationships": [
                    {
                        "type": "RelationshipType",
                        "source": "unique-key-1 or UUID from existing database",
                        "target": "unique-key-1 or UUID from existing database"
                    },
                    // More relationships...
                ]
            }
            
            Guidelines:
            1. Do NOT generate any IDs or UUIDs. Use a unique `key` for each knowledge entity.
            2. Each KnowledgeEntity should have a unique `key`, a meaningful `name`, and a descriptive `description`.
            3. Define the type of each KnowledgeEntity using the following categories: Idea, Project, Document, Page, TextSnippet.
            4. Establish relationships between entities using types like RelatedTo, RelevantTo, SimilarTo.
            5. Use the `source` key to indicate the originating entity and the `target` key to indicate the related entity"
            6. You will be presented with a few existing KnowledgeEntities that are similar to the current ones. They will have an existing UUID. When creating relationships to these entities, use their UUID.
            7. Only create relationships between existing KnowledgeEntities.
            8. Entities that exist already in the database should NOT be created again. If there is only a minor overlap, skip creating a new entity.
            9. A new relationship MUST include a newly created KnowledgeEntity.
            "#;

pub fn get_ingress_analysis_schema() -> Value {
    json!({
      "type": "object",
      "properties": {
        "knowledge_entities": {
          "type": "array",
          "items": {
            "type": "object",
            "properties": {
              "key": { "type": "string" },
              "name": { "type": "string" },
              "description": { "type": "string" },
              "entity_type": {
                "type": "string",
                "enum": ["idea", "project", "document", "page", "textsnippet"]
              }
            },
            "required": ["key", "name", "description", "entity_type"],
            "additionalProperties": false
          }
        },
        "relationships": {
          "type": "array",
          "items": {
            "type": "object",
            "properties": {
              "type": {
                "type": "string",
                "enum": ["RelatedTo", "RelevantTo", "SimilarTo"]
              },
              "source": { "type": "string" },
              "target": { "type": "string" }
            },
            "required": ["type", "source", "target"],
            "additionalProperties": false
          }
        }
      },
      "required": ["knowledge_entities", "relationships"],
      "additionalProperties": false
    })
}
