use axum::async_trait;
use serde::{Deserialize, Serialize};
pub mod file_info;
pub mod knowledge_entity;
pub mod knowledge_relationship;
pub mod text_chunk;
pub mod text_content;
pub mod user;

#[async_trait]
pub trait StoredObject: Serialize + for<'de> Deserialize<'de> {
    fn table_name() -> &'static str;
    fn get_id(&self) -> &str;
}

#[macro_export]
macro_rules! stored_object {
    ($name:ident, $table:expr, {$($(#[$attr:meta])* $field:ident: $ty:ty),*}) => {
        use axum::async_trait;
        use serde::{Deserialize, Deserializer, Serialize};
        use surrealdb::sql::Thing;
        use $crate::storage::types::StoredObject;
        use serde::de::{self, Visitor};
        use std::fmt;

        struct FlexibleIdVisitor;

        impl<'de> Visitor<'de> for FlexibleIdVisitor {
            type Value = String;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string or a Thing")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(value.to_string())
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(value)
            }

            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                // Try to deserialize as Thing
                let thing = Thing::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(thing.id.to_raw())
            }
        }

        fn deserialize_flexible_id<'de, D>(deserializer: D) -> Result<String, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_any(FlexibleIdVisitor)
        }

        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct $name {
            #[serde(deserialize_with = "deserialize_flexible_id")]
            pub id: String,
            $(pub $field: $ty),*
        }

        #[async_trait]
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
