use std::fmt;

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

/// Consume a value without retaining it, using serde_json's normal validation.
/// IgnoredAny bypasses Unicode, number-range and recursion-budget checks.
pub(super) struct Discard;

impl Discard {
    pub(super) fn map<'de, A: MapAccess<'de>>(map: &mut A) -> Result<(), A::Error> {
        while map.next_key::<Self>()?.is_some() {
            map.next_value::<Self>()?;
        }
        Ok(())
    }

    pub(super) fn sequence<'de, A: SeqAccess<'de>>(sequence: &mut A) -> Result<(), A::Error> {
        while sequence.next_element::<Self>()?.is_some() {}
        Ok(())
    }
}

impl<'de> Deserialize<'de> for Discard {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(Self)
    }
}

impl<'de> Visitor<'de> for Discard {
    type Value = Self;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self, E> {
        Ok(self)
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self, E> {
        Ok(self)
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self, E> {
        Ok(self)
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self, E> {
        Ok(self)
    }
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self, E> {
        Ok(self)
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Self, E> {
        Ok(self)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self, A::Error> {
        Self::map(&mut map)?;
        Ok(self)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self, A::Error> {
        Self::sequence(&mut sequence)?;
        Ok(self)
    }
}
