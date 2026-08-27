use crate::semantic_actions::codebook;

pub(in crate::semantic_actions) const SCHEMA_VERSION: i32 = codebook::CURRENT_SCHEMA_VERSION;

pub(in crate::semantic_actions) const ACTION_ATTRIBUTES_FIELD_CODE: i16 = 1;
pub(in crate::semantic_actions) const LINK_ATTRIBUTES_FIELD_CODE: i16 = 2;

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
        zstd_level: 3,
        compression_min_bytes: 64,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::semantic_actions) struct StorageMeta {
    pub schema_version: i32,
    pub cold_fields: ColdFieldMeta,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::semantic_actions) struct ColdFieldMeta {
    pub action_attributes: i16,
    pub link_attributes: i16,
    pub plain_text: i16,
    pub zstd: i16,
    pub compact_plain: i16,
    pub compact_zstd: i16,
}

pub(in crate::semantic_actions) const CURRENT: StorageMeta = StorageMeta {
    schema_version: SCHEMA_VERSION,
    cold_fields: ColdFieldMeta {
        action_attributes: ACTION_ATTRIBUTES_FIELD_CODE,
        link_attributes: LINK_ATTRIBUTES_FIELD_CODE,
        plain_text: ENCODING_PLAIN_TEXT,
        zstd: ENCODING_ZSTD,
        compact_plain: ENCODING_COMPACT_PLAIN,
        compact_zstd: ENCODING_COMPACT_ZSTD,
    },
};
