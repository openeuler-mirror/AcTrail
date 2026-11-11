//! Column conversions shared by the table modules.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::Row;
use rusqlite::types::Type;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Nanoseconds since the Unix epoch, as in the main AcTrail store.
pub(crate) fn encode_time(value: SystemTime) -> i64 {
    let nanos = value
        .duration_since(UNIX_EPOCH)
        .expect("catalog timestamps are after the Unix epoch")
        .as_nanos();
    i64::try_from(nanos).expect("catalog timestamps fit in i64 nanoseconds")
}

pub(crate) fn time(row: &Row<'_>, index: usize) -> rusqlite::Result<SystemTime> {
    let nanos: i64 = row.get(index)?;
    let nanos = u64::try_from(nanos)
        .map_err(|_| conversion(index, Type::Integer, format!("negative timestamp {nanos}")))?;
    Ok(UNIX_EPOCH + Duration::from_nanos(nanos))
}

pub(crate) fn optional_time(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<SystemTime>> {
    match row.get::<_, Option<i64>>(index)? {
        Some(_) => time(row, index).map(Some),
        None => Ok(None),
    }
}

pub(crate) fn to_json(value: &impl Serialize) -> String {
    serde_json::to_string(value).expect("catalog values serialize to JSON")
}

pub(crate) fn json<T: DeserializeOwned>(row: &Row<'_>, index: usize) -> rusqlite::Result<T> {
    let raw: String = row.get(index)?;
    serde_json::from_str(&raw).map_err(|error| conversion(index, Type::Text, error.to_string()))
}

pub(crate) fn optional_json<T: DeserializeOwned>(
    row: &Row<'_>,
    index: usize,
) -> rusqlite::Result<Option<T>> {
    match row.get::<_, Option<String>>(index)? {
        Some(_) => json(row, index).map(Some),
        None => Ok(None),
    }
}

pub(crate) fn text_enum<T>(
    row: &Row<'_>,
    index: usize,
    parse: fn(&str) -> Option<T>,
) -> rusqlite::Result<T> {
    let raw: String = row.get(index)?;
    parse(&raw).ok_or_else(|| conversion(index, Type::Text, format!("unknown value {raw}")))
}

fn conversion(index: usize, kind: Type, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, kind, message.into())
}
