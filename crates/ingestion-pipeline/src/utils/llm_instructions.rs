use common::storage::types::system_prompts::DEFAULT_INGRESS_ANALYSIS_SYSTEM_PROMPT;
use serde_json::json;

pub static INGRESS_ANALYSIS_SYSTEM_MESSAGE: &str = DEFAULT_INGRESS_ANALYSIS_SYSTEM_PROMPT;

pub fn get_ingress_analysis_schema() -> serde_json::Value {
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
                        "entity_type": { "type": "string" }
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
                        "type": { "type": "string" },
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
