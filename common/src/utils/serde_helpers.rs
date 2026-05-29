use chrono::{DateTime, Utc};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use surrealdb::sql::Thing;

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

pub fn serialize_datetime<S>(date: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    Into::<surrealdb::sql::Datetime>::into(*date).serialize(serializer)
}

pub fn deserialize_datetime<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    let dt = surrealdb::sql::Datetime::deserialize(deserializer)?;
    Ok(DateTime::<Utc>::from(dt))
}

pub fn serialize_option_datetime<S>(
    date: &Option<DateTime<Utc>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match date {
        Some(dt) => serializer.serialize_some(&Into::<surrealdb::sql::Datetime>::into(*dt)),
        None => serializer.serialize_none(),
    }
}

pub fn deserialize_option_datetime<'de, D>(
    deserializer: D,
) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<surrealdb::sql::Datetime>::deserialize(deserializer)?;
    Ok(value.map(DateTime::<Utc>::from))
}
