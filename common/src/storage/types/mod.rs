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
    ($name:ident, $table:expr, {$($(#[$attr:meta])* $field:ident: $ty:ty),*}) => {
        use serde::{Deserialize, Deserializer, Serialize};
        use surrealdb::sql::Thing;
        use $crate::storage::types::StoredObject;
        use serde::de::{self, Visitor};
        use std::fmt;
        use chrono::{DateTime, Utc };

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

        pub fn deserialize_flexible_id<'de, D>(deserializer: D) -> Result<String, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_any(FlexibleIdVisitor)
        }

        fn serialize_datetime<S>(date: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Into::<surrealdb::sql::Datetime>::into(*date).serialize(serializer)
        }

        fn deserialize_datetime<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let dt = surrealdb::sql::Datetime::deserialize(deserializer)?;
            Ok(DateTime::<Utc>::from(dt))
        }

        #[allow(dead_code)]
        fn serialize_option_datetime<S>(
            date: &Option<DateTime<Utc>>,
            serializer: S,
        ) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            match date {
                Some(dt) => serializer
                    .serialize_some(&Into::<surrealdb::sql::Datetime>::into(*dt)),
                None => serializer.serialize_none(),
            }
        }

        #[allow(dead_code)]
        fn deserialize_option_datetime<'de, D>(
            deserializer: D,
        ) -> Result<Option<DateTime<Utc>>, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let value = Option::<surrealdb::sql::Datetime>::deserialize(deserializer)?;
            Ok(value.map(DateTime::<Utc>::from))
        }


        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
        pub struct $name {
            #[serde(deserialize_with = "deserialize_flexible_id")]
            pub id: String,
            #[serde(serialize_with = "serialize_datetime", deserialize_with = "deserialize_datetime", default)]
            pub created_at: DateTime<Utc>,
            #[serde(serialize_with = "serialize_datetime", deserialize_with = "deserialize_datetime", default)]
            pub updated_at: DateTime<Utc>,
            $( $(#[$attr])* pub $field: $ty),*
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
