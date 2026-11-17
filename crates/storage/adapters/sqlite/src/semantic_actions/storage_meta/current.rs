//! SQLite storage encoding metadata and compression configuration conversion.

use crate::semantic_actions::codebook;

pub(in crate::semantic_actions) const SCHEMA_VERSION: i32 = codebook::CURRENT_SCHEMA_VERSION;

pub(in crate::semantic_actions) const ENCODING_PLAIN_TEXT: i16 = 0;
pub(in crate::semantic_actions) const ENCODING_ZSTD: i16 = 1;
pub(in crate::semantic_actions) const ENCODING_COMPACT_PLAIN: i16 = 2;
pub(in crate::semantic_actions) const ENCODING_COMPACT_ZSTD: i16 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ColdFieldCompression {
    pub zstd_level: i32,
    pub compression_min_bytes: usize,
}

impl ColdFieldCompression {
    pub const DEFAULT: Self = Self {
        zstd_level: storage_contracts::SQLITE_DEFAULT_COLD_FIELD_ZSTD_LEVEL,
        compression_min_bytes: storage_contracts::SQLITE_DEFAULT_COLD_FIELD_COMPRESSION_MIN_BYTES,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::semantic_actions) struct StorageMeta {
    pub schema_version: i32,
    pub cold_fields: ColdFieldMeta,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::semantic_actions) struct ColdFieldMeta {
    pub plain_text: i16,
    pub zstd: i16,
    pub compact_plain: i16,
    pub compact_zstd: i16,
}

pub(in crate::semantic_actions) const CURRENT: StorageMeta = StorageMeta {
    schema_version: SCHEMA_VERSION,
    cold_fields: ColdFieldMeta {
        plain_text: ENCODING_PLAIN_TEXT,
        zstd: ENCODING_ZSTD,
        compact_plain: ENCODING_COMPACT_PLAIN,
        compact_zstd: ENCODING_COMPACT_ZSTD,
    },
};

impl From<&storage_contracts::SqliteStorageConfig> for ColdFieldCompression {
    fn from(config: &storage_contracts::SqliteStorageConfig) -> Self {
        Self {
            zstd_level: config.cold_field_zstd_level,
            compression_min_bytes: config.cold_field_compression_min_bytes,
        }
    }
}
