pub mod text_content;

#[macro_export]
macro_rules! stored_entity {
    ($name:ident, $table:expr, {$($field:ident: $ty:ty),*}) => {
        use axum::async_trait;
        use serde::{Deserialize, Deserializer, Serialize};
        use surrealdb::sql::Thing;

        fn thing_to_string<'de, D>(deserializer: D) -> Result<String, D::Error>
        where
            D: Deserializer<'de>,
        {
            let thing = Thing::deserialize(deserializer)?;
            Ok(thing.id.to_raw())
        }

        #[async_trait]
        pub trait StoredEntity: Serialize + for<'de> Deserialize<'de> {
            fn table_name() -> &'static str;
            fn get_id(&self) -> &str;
        }

        #[derive(Debug, Serialize, Deserialize)]
        pub struct $name {
            #[serde(deserialize_with = "thing_to_string")]
            pub id: String,
            $(pub $field: $ty),*
        }

        #[async_trait]
        impl StoredEntity for $name {
            fn table_name() -> &'static str {
                $table
            }

            fn get_id(&self) -> &str {
                &self.id
            }
        }
    };
}
