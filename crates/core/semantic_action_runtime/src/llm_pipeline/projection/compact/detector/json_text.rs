use std::fmt;

use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};

use super::super::state::{JsonTextArtifact, PreparedCarrierSnapshot, ResponseTextWitness};
use super::ArtifactFlow;

impl ResponseTextWitness {
    pub(super) fn json_artifact(&self, min_bytes: usize) -> Option<&JsonTextArtifact> {
        self.content
            .json
            .get_or_init(|| JsonTextArtifact::parse(self.text(), min_bytes))
            .as_ref()
    }
}

impl PreparedCarrierSnapshot {
    pub(in crate::llm_pipeline::projection::compact::detector) fn decoded_carrier(
        &self,
        position: usize,
        carrier: &str,
    ) -> Option<&str> {
        self.decoded_text
            .get(position)?
            .get_or_init(|| JsonTextArtifact::decode_carrier(carrier))
            .as_deref()
    }
}

impl JsonTextArtifact {
    /// Decode valid JSON string tokens within a carrier, including embedded
    /// objects in surrounding prose. Retain separators and all other raw text.
    fn decode_carrier(carrier: &str) -> Option<String> {
        let bytes = carrier.as_bytes();
        if !bytes.contains(&b'\\') {
            return None;
        }
        let mut decoded = None;
        let mut copied = 0;
        let mut cursor = 0;
        while let Some(relative) = bytes.get(cursor..)?.iter().position(|byte| *byte == b'"') {
            let start = cursor + relative;
            cursor = start + 1;
            let mut escaped = false;
            while let Some(&byte) = bytes.get(cursor) {
                match byte {
                    b'\\' => {
                        escaped = true;
                        cursor += 2;
                    }
                    b'"' => {
                        cursor += 1;
                        if escaped
                            && let Ok(value) =
                                serde_json::from_str::<String>(&carrier[start..cursor])
                        {
                            let output =
                                decoded.get_or_insert_with(|| String::with_capacity(carrier.len()));
                            output.push_str(&carrier[copied..start + 1]);
                            output.push_str(&value);
                            copied = cursor - 1;
                        }
                        break;
                    }
                    _ => cursor += 1,
                }
            }
            if cursor >= bytes.len() {
                break;
            }
        }
        let mut decoded = decoded?;
        decoded.push_str(&carrier[copied..]);
        Some(decoded)
    }

    pub(super) fn parse(text: &str, min_bytes: usize) -> Option<Self> {
        if !matches!(text.as_bytes().first(), Some(b'{' | b'[')) {
            return None;
        }
        let mut artifact = Self {
            values: Vec::new(),
            bytes: 0,
        };
        let mut deserializer = serde_json::Deserializer::from_str(text);
        TextValues {
            artifact: &mut artifact,
        }
        .deserialize(&mut deserializer)
        .ok()?;
        deserializer.end().ok()?;
        (artifact.bytes >= min_bytes && !artifact.values.is_empty()).then_some(artifact)
    }

    pub(super) fn match_carrier(
        &self,
        carrier: &str,
        carrier_item: usize,
        min_bytes: usize,
        min_coverage_percent: usize,
    ) -> Option<ArtifactFlow> {
        if carrier.len() < min_bytes {
            return None;
        }
        let mut cursor = 0;
        let mut bytes = 0;
        let mut chars = 0;
        let mut remaining = self.bytes;
        for value in &self.values {
            remaining -= value.len();
            if let Some(offset) = carrier.get(cursor..)?.find(value.as_str()) {
                cursor += offset + value.len();
                bytes += value.len();
                chars += value.chars().count();
            } else {
                // Even adopting every remaining value cannot satisfy the
                // thresholds. Avoid scanning the rest of this carrier.
                let possible = bytes + remaining;
                if possible < min_bytes
                    || possible.saturating_mul(100)
                        < self.bytes.saturating_mul(min_coverage_percent)
                {
                    return None;
                }
            }
        }
        (bytes >= min_bytes
            && bytes.saturating_mul(100) >= self.bytes.saturating_mul(min_coverage_percent))
        .then_some(ArtifactFlow {
            chars,
            bytes,
            carrier_item,
            partial_summary: bytes < self.bytes,
        })
    }

    fn push(&mut self, value: String) {
        if value.trim().is_empty() {
            return;
        }
        self.bytes += value.len();
        self.values.push(value);
    }
}

struct TextValues<'a> {
    artifact: &'a mut JsonTextArtifact,
}

impl<'de> DeserializeSeed<'de> for TextValues<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for TextValues<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_map<M>(self, mut map: M) -> Result<(), M::Error>
    where
        M: MapAccess<'de>,
    {
        while map.next_key::<IgnoredAny>()?.is_some() {
            map.next_value_seed(TextValues {
                artifact: self.artifact,
            })?;
        }
        Ok(())
    }

    fn visit_seq<S>(self, mut sequence: S) -> Result<(), S::Error>
    where
        S: SeqAccess<'de>,
    {
        while sequence
            .next_element_seed(TextValues {
                artifact: self.artifact,
            })?
            .is_some()
        {}
        Ok(())
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<(), E> {
        self.artifact.push(value.to_owned());
        Ok(())
    }

    fn visit_str<E>(self, value: &str) -> Result<(), E> {
        self.artifact.push(value.to_owned());
        Ok(())
    }

    fn visit_string<E>(self, value: String) -> Result<(), E> {
        self.artifact.push(value);
        Ok(())
    }

    fn visit_bool<E>(self, _: bool) -> Result<(), E> {
        Ok(())
    }

    fn visit_i64<E>(self, _: i64) -> Result<(), E> {
        Ok(())
    }

    fn visit_u64<E>(self, _: u64) -> Result<(), E> {
        Ok(())
    }

    fn visit_f64<E>(self, _: f64) -> Result<(), E> {
        Ok(())
    }

    fn visit_unit<E>(self) -> Result<(), E> {
        Ok(())
    }
}
