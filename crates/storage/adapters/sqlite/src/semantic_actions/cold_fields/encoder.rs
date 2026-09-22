use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::semantic_actions::attribute_codes::encode_attributes;
use crate::semantic_actions::storage_meta::{ColdFieldCompression, current};

use super::EncodedColdField;

#[derive(Clone)]
pub(crate) struct ColdFieldEncoder {
    config: ColdFieldCompression,
    compressor: Option<Rc<RefCell<zstd::bulk::Compressor<'static>>>>,
}

impl ColdFieldEncoder {
    pub(crate) fn for_read_only() -> Self {
        Self {
            config: ColdFieldCompression::DEFAULT,
            compressor: None,
        }
    }

    pub(crate) fn new(config: ColdFieldCompression) -> Result<Self, rusqlite::Error> {
        let compressor = if config.compression_min_bytes == 0 {
            None
        } else {
            Some(Rc::new(RefCell::new(
                zstd::bulk::Compressor::new(config.zstd_level)
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?,
            )))
        };
        Ok(Self { config, compressor })
    }

    pub(crate) fn zstd_level(&self) -> i32 {
        self.config.zstd_level
    }

    pub(super) fn encode_compact(
        &self,
        attributes: &BTreeMap<String, String>,
    ) -> Result<EncodedColdField, rusqlite::Error> {
        let meta = current().cold_fields;
        let raw = encode_attributes(attributes);
        let uncompressed_bytes =
            i64::try_from(raw.len()).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let (encoding_code, payload) = if self.config.compression_min_bytes != 0
            && raw.len() >= self.config.compression_min_bytes
        {
            let compressed = self
                .compressor
                .as_ref()
                .ok_or(rusqlite::Error::InvalidQuery)?
                .try_borrow_mut()
                .map_err(|_| rusqlite::Error::InvalidQuery)?
                .compress(raw.as_slice())
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            if compressed.len() < raw.len() {
                (meta.compact_zstd, compressed)
            } else {
                (meta.compact_plain, raw)
            }
        } else {
            (meta.compact_plain, raw)
        };
        Ok(EncodedColdField {
            encoding_code,
            uncompressed_bytes,
            payload,
        })
    }
}
