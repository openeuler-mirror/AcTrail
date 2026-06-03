use super::constants::*;
use super::image::ElfImage;
use super::raw::{bounded, read_i64, read_u64, string_at_usize};
use crate::{ToolError, ToolResult};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct DynamicInfo {
    pub(crate) needed: Vec<String>,
    pub(crate) rpath: Vec<String>,
    pub(crate) runpath: Vec<String>,
}

impl ElfImage {
    pub(crate) fn dynamic_info(&self) -> ToolResult<DynamicInfo> {
        let entries = self.dynamic_entries()?;
        let Some(strings) = self.dynamic_strings(&entries)? else {
            return Ok(DynamicInfo::default());
        };
        let mut info = DynamicInfo::default();
        for entry in entries {
            match entry.tag {
                ELF_DYNAMIC_NEEDED => {
                    info.needed
                        .push(dynamic_string(strings, entry.value)?.to_string());
                }
                ELF_DYNAMIC_RPATH => {
                    info.rpath
                        .extend(split_dynamic_path(dynamic_string(strings, entry.value)?));
                }
                ELF_DYNAMIC_RUNPATH => {
                    info.runpath
                        .extend(split_dynamic_path(dynamic_string(strings, entry.value)?));
                }
                _ => {}
            }
        }
        Ok(info)
    }

    fn dynamic_entries(&self) -> ToolResult<Vec<DynamicEntry>> {
        let mut entries = Vec::new();
        for segment in &self.dynamic_segments {
            let data = bounded(&self.data, segment.file_offset, segment.size)?;
            for raw in data.chunks_exact(ELF_DYNAMIC_ENTRY_SIZE) {
                let tag = read_i64(raw, ELF_DYNAMIC_TAG_FIELD)?;
                if tag == ELF_DYNAMIC_NULL {
                    break;
                }
                entries.push(DynamicEntry {
                    tag,
                    value: read_u64(raw, ELF_DYNAMIC_VALUE_FIELD)?,
                });
            }
        }
        Ok(entries)
    }

    fn dynamic_strings<'a>(&'a self, entries: &[DynamicEntry]) -> ToolResult<Option<&'a [u8]>> {
        let mut string_address = None;
        let mut string_size = None;
        for entry in entries {
            match entry.tag {
                ELF_DYNAMIC_STRTAB => string_address = Some(entry.value),
                ELF_DYNAMIC_STRSZ => string_size = Some(entry.value),
                _ => {}
            }
        }
        let Some(address) = string_address else {
            return Ok(None);
        };
        let Some(size) = string_size else {
            return Err(ToolError::new("ELF dynamic string table has no DT_STRSZ"));
        };
        let offset = self.file_offset_for_virtual_address(address)?;
        bounded(&self.data, offset, size).map(Some)
    }
}

struct DynamicEntry {
    tag: i64,
    value: u64,
}

fn dynamic_string(strings: &[u8], offset: u64) -> ToolResult<&str> {
    let offset = usize::try_from(offset)
        .map_err(|_| ToolError::new("ELF dynamic string offset overflow"))?;
    string_at_usize(strings, offset)
}

fn split_dynamic_path(value: &str) -> Vec<String> {
    value
        .split(':')
        .filter(|entry| !entry.is_empty())
        .map(ToString::to_string)
        .collect()
}
