#![allow(clippy::unsafe_derive_deserialize)]
use serde::{Deserialize, Serialize};
pub mod analytics;
pub mod conversation;
pub mod file_info;
pub mod ingestion_payload;
pub mod ingestion_task;
pub mod knowledge_entity;
pub mod knowledge_entity_embedding;
pub mod knowledge_relationship;
pub mod message;
pub mod scratchpad;
pub mod serde_helpers;
pub mod system_prompts;
pub mod system_settings;
pub mod text_chunk;
pub mod text_chunk_embedding;
pub mod text_content;
pub mod user;

pub trait StoredObject: Serialize + for<'de> Deserialize<'de> {
    fn table_name() -> &'static str;
    fn get_id(&self) -> &str;
}

#[macro_export]
macro_rules! stored_object {
    ($(#[$struct_attr:meta])* $name:ident, $table:expr, {$($(#[$field_attr:meta])* $field:ident: $ty:ty),*}) => {
        use serde::{Deserialize, Serialize};
        use $crate::storage::types::StoredObject;
        #[allow(unused_imports)]
        use $crate::storage::types::serde_helpers::{
            deserialize_flexible_id, serialize_datetime, deserialize_datetime,
            serialize_option_datetime, deserialize_option_datetime,
        };
        use chrono::{DateTime, Utc };

        $(#[$struct_attr])*
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
        pub struct $name {
            #[serde(deserialize_with = "deserialize_flexible_id")]
            pub id: String,
            #[serde(serialize_with = "serialize_datetime", deserialize_with = "deserialize_datetime", default)]
            pub created_at: DateTime<Utc>,
            #[serde(serialize_with = "serialize_datetime", deserialize_with = "deserialize_datetime", default)]
            pub updated_at: DateTime<Utc>,
            $( $(#[$field_attr])* pub $field: $ty),*
        }

        impl StoredObject for $name {
            fn table_name() -> &'static str {
                $table
            }

            fn get_id(&self) -> &str {
                &self.id
            }
        }
    };
}
