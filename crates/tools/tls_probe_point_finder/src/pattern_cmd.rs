//! Maintenance command for extracting byte patterns from verified addresses.

use crate::args::{PatternArgs, require_arch};
use crate::binary::resolve_entry_elf;
use crate::detect::{OffsetAddressReport, TargetReport};
use crate::elf::ElfImage;
use crate::{ToolError, ToolResult};

pub(crate) struct PatternReport {
    pub(crate) target: TargetReport,
    pub(crate) address: String,
    pub(crate) file_offset: String,
    pub(crate) length: String,
    pub(crate) match_count: usize,
    pub(crate) pattern_hex: String,
    pub(crate) matches: Vec<OffsetAddressReport>,
}

pub(crate) fn run(args: PatternArgs) -> ToolResult<PatternReport> {
    let binary = resolve_entry_elf(&args.binary)?;
    let image = ElfImage::parse(&binary)?;
    require_arch(image.arch(), args.arch, image.path())?;
    let file_offset = image.file_offset_for_virtual_address(args.address)?;
    let end = file_offset
        .checked_add(args.length as u64)
        .ok_or_else(|| ToolError::new("pattern range overflows u64"))?;
    if usize::try_from(end).map_or(true, |end| end > image.data().len()) {
        return Err(ToolError::new(format!(
            "pattern range exceeds file size at 0x{file_offset:x}"
        )));
    }
    if args.length == 0 {
        return Err(ToolError::new("pattern length must be non-zero"));
    }
    let start = usize::try_from(file_offset)
        .map_err(|_| ToolError::new("pattern file offset overflows usize"))?;
    let pattern = &image.data()[start..start + args.length];
    let matches = find_all(image.data(), pattern)?;
    let shown_matches = matches
        .iter()
        .copied()
        .take(args.match_limit)
        .map(|file_offset| {
            Ok(OffsetAddressReport {
                file_offset: format!("0x{file_offset:x}"),
                virtual_address: format!(
                    "0x{:x}",
                    image.virtual_address_for_file_offset(file_offset as u64)?
                ),
            })
        })
        .collect::<ToolResult<Vec<_>>>()?;
    Ok(PatternReport {
        target: TargetReport {
            binary: image.path().display().to_string(),
            architecture: image.arch().as_str().to_string(),
            build_id: image.build_id().unwrap_or("not_found").to_string(),
        },
        address: format!("0x{:x}", args.address),
        file_offset: format!("0x{file_offset:x}"),
        length: format!("0x{:x}", args.length),
        match_count: matches.len(),
        pattern_hex: pattern_hex(pattern),
        matches: shown_matches,
    })
}

pub(crate) fn find_all(data: &[u8], pattern: &[u8]) -> ToolResult<Vec<usize>> {
    if pattern.is_empty() {
        return Err(ToolError::new("pattern must not be empty"));
    }
    let mut offsets = Vec::new();
    let mut start = 0_usize;
    while start <= data.len().saturating_sub(pattern.len()) {
        let Some(relative) = data[start..].iter().position(|byte| *byte == pattern[0]) else {
            break;
        };
        let offset = start + relative;
        if data.get(offset..offset + pattern.len()) == Some(pattern) {
            offsets.push(offset);
        }
        start = offset + 1;
    }
    Ok(offsets)
}

fn pattern_hex(pattern: &[u8]) -> String {
    pattern
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}
