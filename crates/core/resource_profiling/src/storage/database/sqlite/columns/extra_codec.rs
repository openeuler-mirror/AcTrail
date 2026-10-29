use crate::storage::ResourceStorageError;
use std::borrow::Cow;

/// Only unsigned decimal strings that survive formatting unchanged use varints.
/// Leading zeroes, signs, whitespace and values above u64::MAX stay as UTF-8.
pub(super) fn canonical_integer(value: &str) -> Option<u64> {
    value.parse::<u64>().ok().filter(|v| v.to_string() == value)
}

pub(super) fn encode(
    source: Option<(&str, &str)>,
    values: &[(u64, &str)],
    removed: &[u64],
) -> Option<Vec<u8>> {
    if source.is_none() && values.is_empty() && removed.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    if let Some((scope, subject)) = source {
        put_string(&mut out, scope);
        put_string(&mut out, subject);
    }
    put_varint(
        &mut out,
        (values.len() as u64) << 1 | u64::from(!removed.is_empty()),
    );
    for &(key, value) in values {
        let integer = canonical_integer(value);
        put_varint(&mut out, key << 1 | u64::from(integer.is_some()));
        if let Some(integer) = integer {
            put_varint(&mut out, integer);
        } else {
            put_string(&mut out, value);
        }
    }
    if !removed.is_empty() {
        put_varint(&mut out, removed.len() as u64);
        for &key in removed {
            put_varint(&mut out, key);
        }
    }
    Some(out)
}

pub(super) struct Extra<'a> {
    pub source: Option<(&'a str, &'a str)>,
    pub values: Vec<(u64, Cow<'a, str>)>,
    pub removed: Vec<u64>,
}

pub(super) fn decode(
    bytes: &[u8],
    source_override: bool,
) -> Result<Extra<'_>, ResourceStorageError> {
    let mut input = Input(bytes);
    let source = if source_override {
        Some((input.string()?, input.string()?))
    } else {
        None
    };
    let header = input.varint()?;
    let mut values = Vec::new();
    for _ in 0..header >> 1 {
        let key = input.varint()?;
        let value = if key & 1 != 0 {
            Cow::Owned(input.varint()?.to_string())
        } else {
            Cow::Borrowed(input.string()?)
        };
        values.push((key >> 1, value));
    }
    let mut removed = Vec::new();
    if header & 1 != 0 {
        for _ in 0..input.varint()? {
            removed.push(input.varint()?);
        }
    }
    if !input.0.is_empty() {
        return Err(invalid_extra());
    }
    Ok(Extra {
        source,
        values,
        removed,
    })
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn put_string(out: &mut Vec<u8>, value: &str) {
    put_varint(out, value.len() as u64);
    out.extend_from_slice(value.as_bytes());
}

struct Input<'a>(&'a [u8]);

impl<'a> Input<'a> {
    fn varint(&mut self) -> Result<u64, ResourceStorageError> {
        let mut value = 0;
        for shift in (0..=63).step_by(7) {
            let (&byte, rest) = self.0.split_first().ok_or_else(invalid_extra)?;
            self.0 = rest;
            if shift == 63 && byte > 1 {
                return Err(invalid_extra());
            }
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(invalid_extra())
    }

    fn string(&mut self) -> Result<&'a str, ResourceStorageError> {
        let len = usize::try_from(self.varint()?).map_err(|_| invalid_extra())?;
        let bytes = self.0.get(..len).ok_or_else(invalid_extra)?;
        let value = std::str::from_utf8(bytes).map_err(|_| invalid_extra())?;
        self.0 = &self.0[len..];
        Ok(value)
    }
}

fn invalid_extra() -> ResourceStorageError {
    ResourceStorageError("invalid resource metadata extra".into())
}
