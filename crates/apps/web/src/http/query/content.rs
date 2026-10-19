use storage_core::ContentNodeQuery;

use super::{optional_query_param, required_query_usize};

#[derive(Clone)]
pub(in crate::http) struct ContentLimits {
    max_bytes: usize,
    max_node_items: usize,
    preview_bytes: usize,
    block_read_budget: usize,
}

impl ContentLimits {
    pub(in crate::http) fn load() -> Result<Self, String> {
        Ok(Self {
            max_bytes: Self::setting("ACTRAIL_WEB_CONTENT_MAX_BYTES", 16 * 1024 * 1024)?,
            max_node_items: Self::setting("ACTRAIL_WEB_CONTENT_NODE_MAX_ITEMS", 250)?,
            preview_bytes: Self::setting("ACTRAIL_WEB_CONTENT_NODE_PREVIEW_BYTES", 64 * 1024)?,
            block_read_budget: Self::setting(
                "ACTRAIL_WEB_CONTENT_NODE_BLOCK_BYTES",
                8 * 1024 * 1024,
            )?,
        })
    }

    fn setting(name: &str, default: usize) -> Result<usize, String> {
        match std::env::var(name) {
            Ok(raw) => raw
                .parse::<usize>()
                .ok()
                .filter(|value| *value > 0 && *value < i64::MAX as usize)
                .ok_or_else(|| format!("{name} must be a positive integer below i64::MAX")),
            Err(std::env::VarError::NotPresent) => Ok(default),
            Err(error) => Err(format!("invalid {name}: {error}")),
        }
    }

    pub(in crate::http) fn http(
        &self,
        query: &str,
    ) -> Result<storage_core::HttpContentQuery, String> {
        let offset = required_query_usize(query, "offset")?;
        let max_bytes = if optional_query_param(query, "max_bytes")?.is_some() {
            self.prefix(query)?
        } else {
            self.preview_bytes.min(self.max_bytes)
        };
        Ok(storage_core::HttpContentQuery {
            offset,
            max_bytes,
            read_budget: self.block_read_budget,
        })
    }

    pub(in crate::http) fn prefix(&self, query: &str) -> Result<usize, String> {
        let bytes = required_query_usize(query, "max_bytes")?;
        if bytes == 0 || bytes > self.max_bytes {
            return Err(format!(
                "max_bytes must be between 1 and {}",
                self.max_bytes
            ));
        }
        Ok(bytes)
    }

    pub(in crate::http) fn node(&self, query: &str) -> Result<ContentNodeQuery, String> {
        let pointer = optional_query_param(query, "pointer")?.unwrap_or_default();
        if !pointer.is_empty() && !pointer.starts_with('/') {
            return Err("invalid query parameter pointer: JSON Pointer must start with /".into());
        }
        let offset = required_query_usize(query, "offset")?;
        let limit = required_query_usize(query, "limit")?;
        if limit == 0 || limit > self.max_node_items {
            return Err(format!(
                "node limit must be between 1 and {}",
                self.max_node_items
            ));
        }
        Ok(ContentNodeQuery {
            pointer,
            offset,
            limit,
            preview_bytes: self.preview_bytes,
            block_read_budget: self.block_read_budget,
        })
    }
}
