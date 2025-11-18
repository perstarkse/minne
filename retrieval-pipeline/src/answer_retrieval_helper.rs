use common::storage::types::system_prompts::DEFAULT_QUERY_SYSTEM_PROMPT;
use serde_json::{json, Value};

pub static QUERY_SYSTEM_PROMPT: &str = DEFAULT_QUERY_SYSTEM_PROMPT;

pub fn get_query_response_schema() -> Value {
    json!({
       "type": "object",
       "properties": {
           "answer": { "type": "string" },
           "references": {
               "type": "array",
               "items": {
                   "type": "object",
                   "properties": {
                       "reference": { "type": "string" },
                   },
               "required": ["reference"],
               "additionalProperties": false,
               }
           }
       },
       "required": ["answer", "references"],
       "additionalProperties": false
    })
}
