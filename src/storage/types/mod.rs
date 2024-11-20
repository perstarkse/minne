use axum::async_trait;
use serde::{Deserialize, Serialize};
pub mod text_chunk;
pub mod text_content;

#[async_trait]
pub trait StoredObject: Serialize + for<'de> Deserialize<'de> {
    fn table_name() -> &'static str;
    fn get_id(&self) -> &str;
}

#[macro_export]
macro_rules! stored_object {
    ($name:ident, $table:expr, {$($field:ident: $ty:ty),*}) => {
        use axum::async_trait;
        use serde::{Deserialize, Deserializer, Serialize};
        use surrealdb::sql::Thing;
        use $crate::storage::types::StoredObject;

        fn thing_to_string<'de, D>(deserializer: D) -> Result<String, D::Error>
        where
            D: Deserializer<'de>,
        {
            let thing = Thing::deserialize(deserializer)?;
            Ok(thing.id.to_raw())
        }


        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct $name {
            #[serde(deserialize_with = "thing_to_string")]
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
