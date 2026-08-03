use crate::elf::ElfImage;
use crate::pattern_search::ExactPatternSearch;
use crate::{ToolError, ToolResult};

pub(crate) struct StaticPatternDetection {
    pub(crate) arch_label: &'static str,
    pub(crate) matches: Vec<PatternMatches>,
    pub(crate) offsets: Vec<DetectedOffset>,
}

pub(crate) struct PatternMatches {
    pub(crate) pattern_id: &'static str,
    pub(crate) symbol: &'static str,
    pub(crate) pattern_length: usize,
    pub(crate) match_count: usize,
    pub(crate) shown_matches: Vec<OffsetAddress>,
}

pub(crate) struct DetectedOffset {
    pub(crate) symbol: &'static str,
    pub(crate) file_offset: usize,
    pub(crate) virtual_address: u64,
}

pub(crate) struct OffsetAddress {
    pub(crate) file_offset: usize,
    pub(crate) virtual_address: u64,
}

pub(crate) struct StaticPatternSupport;

impl StaticPatternSupport {
    pub(crate) fn find_all(data: &[u8], pattern: &[u8]) -> Vec<usize> {
        ExactPatternSearch::new(pattern).map_or_else(Vec::new, |search| search.find_all(data))
    }

    pub(crate) fn find_all_executable(image: &ElfImage, pattern: &[u8]) -> ToolResult<Vec<usize>> {
        let ranges = image.executable_file_ranges()?;
        Ok(ExactPatternSearch::new(pattern)
            .map_or_else(Vec::new, |search| search.find_all_in_file_ranges(&ranges)))
    }

    pub(crate) fn contains(data: &[u8], pattern: &[u8]) -> bool {
        !Self::find_all(data, pattern).is_empty()
    }

    pub(crate) fn matches_at(data: &[u8], offset: usize, pattern: &[u8]) -> bool {
        data.get(offset..offset + pattern.len()) == Some(pattern)
    }

    pub(crate) fn require_single(offsets: &[usize], symbol: &str) -> ToolResult<usize> {
        if offsets.len() == 1 {
            Ok(offsets[0])
        } else {
            Err(ToolError::new(format!(
                "BoringSSL {symbol} pattern match count={}",
                offsets.len()
            )))
        }
    }

    pub(crate) fn pattern_matches(
        image: &ElfImage,
        pattern_id: &'static str,
        symbol: &'static str,
        pattern: &[u8],
        offsets: &[usize],
        match_limit: usize,
    ) -> ToolResult<PatternMatches> {
        Ok(PatternMatches {
            pattern_id,
            symbol,
            pattern_length: pattern.len(),
            match_count: offsets.len(),
            shown_matches: offsets
                .iter()
                .copied()
                .take(match_limit)
                .map(|file_offset| {
                    Ok(OffsetAddress {
                        file_offset,
                        virtual_address: image
                            .virtual_address_for_file_offset(file_offset as u64)?,
                    })
                })
                .collect::<ToolResult<Vec<_>>>()?,
        })
    }

    pub(crate) fn offsets_with_addresses(
        image: &ElfImage,
        offsets: &[(&'static str, usize)],
    ) -> ToolResult<Vec<DetectedOffset>> {
        offsets
            .iter()
            .map(|(symbol, file_offset)| {
                Ok(DetectedOffset {
                    symbol,
                    file_offset: *file_offset,
                    virtual_address: image.virtual_address_for_file_offset(*file_offset as u64)?,
                })
            })
            .collect()
    }
}
