use std::fs;
use std::path::{Path, PathBuf};

use crate::{ToolError, ToolResult};

use super::constants::*;
use super::raw::{
    bounded, bounded_usize, checked_table_offset, hex_bytes, read_u16, read_u32, read_u64,
    string_at,
};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum Arch {
    Aarch64,
    X86_64,
}

impl Arch {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Aarch64 => "aarch64",
            Self::X86_64 => "x86_64",
        }
    }
}

pub(crate) struct ElfImage {
    pub(super) path: PathBuf,
    pub(super) data: Vec<u8>,
    pub(super) arch: Arch,
    pub(super) build_id: Option<String>,
    pub(super) load_segments: Vec<LoadSegment>,
    pub(super) dynamic_segments: Vec<SegmentRange>,
    pub(super) sections: Vec<ElfSection>,
}

impl ElfImage {
    pub(crate) fn parse(path: &Path) -> ToolResult<Self> {
        let data = fs::read(path)
            .map_err(|error| ToolError::new(format!("cannot read {}: {error}", path.display())))?;
        validate_header(&data)?;
        let arch = parse_arch(&data)?;
        let load_segments = parse_load_segments(&data)?;
        let dynamic_segments = parse_dynamic_segments(&data)?;
        let sections = parse_sections(&data)?;
        let build_id = match parse_program_build_id(&data)? {
            Some(found) => Some(found),
            None => parse_section_build_id(&data, &sections)?,
        };
        Ok(Self {
            path: path.to_path_buf(),
            data,
            arch,
            build_id,
            load_segments,
            dynamic_segments,
            sections,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn data(&self) -> &[u8] {
        &self.data
    }

    pub(crate) fn arch(&self) -> Arch {
        self.arch
    }

    pub(crate) fn build_id(&self) -> Option<&str> {
        self.build_id.as_deref()
    }

    pub(crate) fn file_offset_for_virtual_address(&self, virtual_address: u64) -> ToolResult<u64> {
        self.load_segments
            .iter()
            .find_map(|segment| segment.file_offset_for(virtual_address))
            .ok_or_else(|| {
                ToolError::new(format!(
                    "virtual address 0x{virtual_address:x} is not inside a LOAD segment"
                ))
            })
    }

    pub(crate) fn virtual_address_for_file_offset(&self, file_offset: u64) -> ToolResult<u64> {
        self.load_segments
            .iter()
            .find_map(|segment| segment.virtual_address_for(file_offset))
            .ok_or_else(|| {
                ToolError::new(format!(
                    "file offset 0x{file_offset:x} is not inside a LOAD segment"
                ))
            })
    }

    pub(crate) fn section_data(&self, name: &str) -> ToolResult<Option<&[u8]>> {
        let Some(section) = self.sections.iter().find(|section| section.name == name) else {
            return Ok(None);
        };
        bounded(&self.data, section.file_offset, section.size).map(Some)
    }

    pub(crate) fn section_virtual_address(&self, name: &str) -> Option<u64> {
        self.sections
            .iter()
            .find(|section| section.name == name)
            .map(|section| section.virtual_address)
    }
}

#[derive(Clone)]
pub(super) struct LoadSegment {
    file_offset: u64,
    virtual_address: u64,
    file_size: u64,
}

impl LoadSegment {
    fn file_offset_for(&self, virtual_address: u64) -> Option<u64> {
        if virtual_address < self.virtual_address {
            return None;
        }
        let relative = virtual_address - self.virtual_address;
        if relative >= self.file_size {
            return None;
        }
        self.file_offset.checked_add(relative)
    }

    fn virtual_address_for(&self, file_offset: u64) -> Option<u64> {
        if file_offset < self.file_offset {
            return None;
        }
        let relative = file_offset - self.file_offset;
        if relative >= self.file_size {
            return None;
        }
        self.virtual_address.checked_add(relative)
    }
}

#[derive(Clone)]
pub(super) struct SegmentRange {
    pub(super) file_offset: u64,
    pub(super) size: u64,
}

#[derive(Clone)]
pub(super) struct ElfSection {
    pub(super) name: String,
    pub(super) section_type: u32,
    pub(super) virtual_address: u64,
    pub(super) file_offset: u64,
    pub(super) size: u64,
    pub(super) link: u32,
    pub(super) entry_size: u64,
}

fn validate_header(data: &[u8]) -> ToolResult<()> {
    if data.len() < ELF64_HEADER_SIZE || &data[0..ELF_MAGIC.len()] != ELF_MAGIC {
        return Err(ToolError::new("target is not an ELF file"));
    }
    if data[ELF_CLASS_OFFSET] != ELF_CLASS_64 || data[ELF_DATA_OFFSET] != ELF_DATA_LITTLE_ENDIAN {
        return Err(ToolError::new("target must be ELF64 little-endian"));
    }
    Ok(())
}

fn parse_arch(data: &[u8]) -> ToolResult<Arch> {
    match read_u16(data, ELF_MACHINE_FIELD)? {
        ELF_MACHINE_AARCH64 => Ok(Arch::Aarch64),
        ELF_MACHINE_X86_64 => Ok(Arch::X86_64),
        machine => Err(ToolError::new(format!(
            "unsupported ELF machine 0x{machine:x}"
        ))),
    }
}

fn parse_load_segments(data: &[u8]) -> ToolResult<Vec<LoadSegment>> {
    let mut segments = Vec::new();
    for_program_header(data, |header| {
        if read_u32(header, ELF_PROGRAM_HEADER_TYPE_FIELD)? == ELF_PROGRAM_HEADER_LOAD {
            segments.push(LoadSegment {
                file_offset: read_u64(header, ELF_PROGRAM_HEADER_FILE_OFFSET_FIELD)?,
                virtual_address: read_u64(header, ELF_PROGRAM_HEADER_VADDR_FIELD)?,
                file_size: read_u64(header, ELF_PROGRAM_HEADER_FILE_SIZE_FIELD)?,
            });
        }
        Ok(())
    })?;
    if segments.is_empty() {
        return Err(ToolError::new("target has no ELF LOAD segments"));
    }
    Ok(segments)
}

fn parse_dynamic_segments(data: &[u8]) -> ToolResult<Vec<SegmentRange>> {
    let mut segments = Vec::new();
    for_program_header(data, |header| {
        if read_u32(header, ELF_PROGRAM_HEADER_TYPE_FIELD)? == ELF_PROGRAM_HEADER_DYNAMIC {
            segments.push(SegmentRange {
                file_offset: read_u64(header, ELF_PROGRAM_HEADER_FILE_OFFSET_FIELD)?,
                size: read_u64(header, ELF_PROGRAM_HEADER_FILE_SIZE_FIELD)?,
            });
        }
        Ok(())
    })?;
    Ok(segments)
}

fn for_program_header<F>(data: &[u8], mut visit: F) -> ToolResult<()>
where
    F: FnMut(&[u8]) -> ToolResult<()>,
{
    let table_offset = read_u64(data, ELF_PROGRAM_HEADER_TABLE_OFFSET_FIELD)?;
    let entry_size = read_u16(data, ELF_PROGRAM_HEADER_ENTRY_SIZE_FIELD)? as u64;
    let count = read_u16(data, ELF_PROGRAM_HEADER_COUNT_FIELD)? as u64;
    for index in 0..count {
        let offset = checked_table_offset(
            table_offset,
            entry_size,
            index,
            "ELF program-header overflow",
        )?;
        visit(bounded(data, offset, entry_size)?)?;
    }
    Ok(())
}

fn parse_sections(data: &[u8]) -> ToolResult<Vec<ElfSection>> {
    let table_offset = read_u64(data, ELF_SECTION_HEADER_TABLE_OFFSET_FIELD)?;
    let entry_size = read_u16(data, ELF_SECTION_HEADER_TABLE_ENTRY_SIZE_FIELD)? as u64;
    let count = read_u16(data, ELF_SECTION_HEADER_COUNT_FIELD)? as u64;
    let name_table_index = read_u16(data, ELF_SECTION_NAME_TABLE_INDEX_FIELD)? as usize;
    if table_offset == 0 || entry_size == 0 || count == 0 {
        return Ok(Vec::new());
    }
    let mut raw_sections = Vec::new();
    for index in 0..count {
        let offset = checked_table_offset(
            table_offset,
            entry_size,
            index,
            "ELF section-header overflow",
        )?;
        let header = bounded(data, offset, entry_size)?;
        raw_sections.push((
            read_u32(header, ELF_SECTION_HEADER_NAME_FIELD)?,
            read_u32(header, ELF_SECTION_HEADER_TYPE_FIELD)?,
            read_u64(header, ELF_SECTION_HEADER_ADDR_FIELD)?,
            read_u64(header, ELF_SECTION_HEADER_FILE_OFFSET_FIELD)?,
            read_u64(header, ELF_SECTION_HEADER_SIZE_FIELD)?,
            read_u32(header, ELF_SECTION_HEADER_LINK_FIELD)?,
            read_u64(header, ELF_SECTION_ENTRY_SIZE_FIELD)?,
        ));
    }
    let names_section = raw_sections.get(name_table_index);
    let names = match names_section {
        Some((_, _, _, offset, size, _, _)) => Some(bounded(data, *offset, *size)?),
        None => None,
    };
    raw_sections
        .into_iter()
        .map(
            |(name_offset, section_type, virtual_address, file_offset, size, link, entry_size)| {
                let name = match names {
                    Some(table) => string_at(table, name_offset)?.unwrap_or("").to_string(),
                    None => String::new(),
                };
                Ok(ElfSection {
                    name,
                    section_type,
                    virtual_address,
                    file_offset,
                    size,
                    link,
                    entry_size,
                })
            },
        )
        .collect()
}

fn parse_program_build_id(data: &[u8]) -> ToolResult<Option<String>> {
    let mut build_id = None;
    for_program_header(data, |header| {
        if read_u32(header, ELF_PROGRAM_HEADER_TYPE_FIELD)? == ELF_PROGRAM_HEADER_NOTE {
            let offset = read_u64(header, ELF_PROGRAM_HEADER_FILE_OFFSET_FIELD)?;
            let size = read_u64(header, ELF_PROGRAM_HEADER_FILE_SIZE_FIELD)?;
            if let Some(found) = parse_build_id_note(bounded(data, offset, size)?)? {
                build_id = Some(found);
            }
        }
        Ok(())
    })?;
    Ok(build_id)
}

fn parse_section_build_id(data: &[u8], sections: &[ElfSection]) -> ToolResult<Option<String>> {
    for section in sections
        .iter()
        .filter(|section| section.section_type == ELF_SECTION_NOTE)
    {
        if let Some(found) = parse_build_id_note(bounded(data, section.file_offset, section.size)?)?
        {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn parse_build_id_note(data: &[u8]) -> ToolResult<Option<String>> {
    let mut offset = 0_usize;
    while offset + ELF_NOTE_HEADER_SIZE <= data.len() {
        let name_size = read_u32(data, offset + ELF_NOTE_NAME_SIZE_FIELD)? as usize;
        let description_size = read_u32(data, offset + ELF_NOTE_DESCRIPTION_SIZE_FIELD)? as usize;
        let note_type = read_u32(data, offset + ELF_NOTE_TYPE_FIELD)?;
        offset += ELF_NOTE_HEADER_SIZE;
        let name = bounded_usize(data, offset, name_size)?;
        offset = align_note_offset(offset, name_size)?;
        let description = bounded_usize(data, offset, description_size)?;
        offset = align_note_offset(offset, description_size)?;
        if note_type == ELF_NOTE_GNU_BUILD_ID && name == ELF_NOTE_NAME_GNU {
            return Ok(Some(hex_bytes(description)));
        }
    }
    Ok(None)
}

fn align_note_offset(offset: usize, size: usize) -> ToolResult<usize> {
    offset
        .checked_add(size)
        .and_then(|value| value.checked_add(ELF_NOTE_ALIGNMENT - 1))
        .map(|value| value & !(ELF_NOTE_ALIGNMENT - 1))
        .ok_or_else(|| ToolError::new("ELF note overflow"))
}
