//! ELF helpers for executable TLS uprobe offsets.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::loader::LoaderError;

const ELF64_HEADER_SIZE: usize = 64;
const ELF_MAGIC: &[u8; 4] = b"\x7fELF";
const ELF_CLASS_OFFSET: usize = 4;
const ELF_DATA_OFFSET: usize = 5;
const ELF_CLASS_64: u8 = 2;
const ELF_DATA_LITTLE_ENDIAN: u8 = 1;
const ELF_PROGRAM_HEADER_TABLE_OFFSET_FIELD: usize = 32;
const ELF_SECTION_HEADER_TABLE_OFFSET_FIELD: usize = 40;
const ELF_PROGRAM_HEADER_ENTRY_SIZE_FIELD: usize = 54;
const ELF_PROGRAM_HEADER_COUNT_FIELD: usize = 56;
const ELF_SECTION_HEADER_TABLE_ENTRY_SIZE_FIELD: usize = 58;
const ELF_SECTION_HEADER_COUNT_FIELD: usize = 60;
const ELF_PROGRAM_HEADER_LOAD: u32 = 1;
const ELF_PROGRAM_HEADER_NOTE: u32 = 4;
const ELF_PROGRAM_HEADER_TYPE_FIELD: usize = 0;
const ELF_PROGRAM_HEADER_FLAGS_FIELD: usize = 4;
const ELF_PROGRAM_HEADER_FILE_OFFSET_FIELD: usize = 8;
const ELF_PROGRAM_HEADER_VADDR_FIELD: usize = 16;
const ELF_PROGRAM_HEADER_FILE_SIZE_FIELD: usize = 32;
const ELF_PROGRAM_HEADER_EXECUTABLE: u32 = 1;
const ELF_SECTION_HEADER_TYPE_FIELD: usize = 4;
const ELF_SECTION_HEADER_FILE_OFFSET_FIELD: usize = 24;
const ELF_SECTION_HEADER_SIZE_FIELD: usize = 32;
const ELF_SECTION_HEADER_LINK_FIELD: usize = 40;
const ELF_SECTION_ENTRY_SIZE_FIELD: usize = 56;
const ELF_SECTION_DYNSYM: u32 = 11;
const ELF_SECTION_SYMTAB: u32 = 2;
const ELF_SYMBOL_NAME_FIELD: usize = 0;
const ELF_SYMBOL_SECTION_INDEX_FIELD: usize = 6;
const ELF_SYMBOL_VALUE_FIELD: usize = 8;
const ELF_SYMBOL_TABLE_ENTRY_SIZE: usize = 24;
const ELF_SECTION_UNDEFINED: u16 = 0;
const ELF_NOTE_GNU_BUILD_ID: u32 = 3;
const ELF_NOTE_NAME_GNU: &[u8] = b"GNU\0";
const ELF_NOTE_HEADER_SIZE: usize = 12;
const ELF_NOTE_NAME_SIZE_FIELD: usize = 0;
const ELF_NOTE_DESCRIPTION_SIZE_FIELD: usize = 4;
const ELF_NOTE_TYPE_FIELD: usize = 8;
const ELF_NOTE_ALIGNMENT: usize = 4;

pub(super) fn resolve_executable_symbol_offsets(
    binary_path: &Path,
    required_symbols: &[&str],
    label: &str,
) -> Result<BTreeMap<String, usize>, LoaderError> {
    resolve_symbol_offsets(
        binary_path,
        required_symbols,
        label,
        "payload_tls_binary_path",
    )
}

pub(super) fn resolve_shared_library_symbol_offsets(
    library_path: &Path,
    required_symbols: &[&str],
    label: &str,
) -> Result<BTreeMap<String, usize>, LoaderError> {
    resolve_symbol_offsets(
        library_path,
        required_symbols,
        label,
        "payload_tls_library_path",
    )
}

fn resolve_symbol_offsets(
    binary_path: &Path,
    required_symbols: &[&str],
    label: &str,
    stage: &'static str,
) -> Result<BTreeMap<String, usize>, LoaderError> {
    let binary =
        fs::read(binary_path).map_err(|error| LoaderError::new(stage, error.to_string()))?;
    let elf = ElfImage::parse(&binary)?;
    elf.resolve_symbol_offsets(&binary, required_symbols, label, stage)
}

pub(super) struct ElfImage {
    build_id: Option<String>,
    load_segments: Vec<LoadSegment>,
}

impl ElfImage {
    pub(super) fn parse(data: &[u8]) -> Result<Self, LoaderError> {
        validate_header(data)?;
        let program_header_offset = read_u64(data, ELF_PROGRAM_HEADER_TABLE_OFFSET_FIELD)?;
        let program_header_entry_size = read_u16(data, ELF_PROGRAM_HEADER_ENTRY_SIZE_FIELD)? as u64;
        let program_header_count = read_u16(data, ELF_PROGRAM_HEADER_COUNT_FIELD)? as u64;
        let mut load_segments = Vec::new();
        let mut build_id = None;
        for index in 0..program_header_count {
            let offset = checked_table_offset(
                program_header_offset,
                program_header_entry_size,
                index,
                "ELF program-header overflow",
            )?;
            let header = bounded(data, offset, program_header_entry_size)?;
            let header_type = read_u32(header, ELF_PROGRAM_HEADER_TYPE_FIELD)?;
            let flags = read_u32(header, ELF_PROGRAM_HEADER_FLAGS_FIELD)?;
            let segment_offset = read_u64(header, ELF_PROGRAM_HEADER_FILE_OFFSET_FIELD)?;
            let virtual_address = read_u64(header, ELF_PROGRAM_HEADER_VADDR_FIELD)?;
            let file_size = read_u64(header, ELF_PROGRAM_HEADER_FILE_SIZE_FIELD)?;
            if header_type == ELF_PROGRAM_HEADER_LOAD {
                load_segments.push(LoadSegment {
                    file_offset: segment_offset,
                    virtual_address,
                    file_size,
                    executable: flags & ELF_PROGRAM_HEADER_EXECUTABLE != 0,
                });
            } else if header_type == ELF_PROGRAM_HEADER_NOTE {
                let note = bounded(data, segment_offset, file_size)?;
                if let Some(found) = parse_build_id_note(note)? {
                    build_id = Some(found);
                }
            }
        }
        if load_segments.is_empty() {
            return Err(LoaderError::new(
                "payload_tls_binary_path",
                "target executable has no ELF load segments",
            ));
        }
        Ok(Self {
            build_id,
            load_segments,
        })
    }

    pub(super) fn build_id(&self) -> Option<&str> {
        self.build_id.as_deref()
    }

    pub(super) fn executable_file_offset(
        &self,
        virtual_address: u64,
        stage: &'static str,
        label: &str,
    ) -> Result<usize, LoaderError> {
        self.load_segments
            .iter()
            .find_map(|segment| segment.file_offset_for(virtual_address))
            .ok_or_else(|| {
                LoaderError::new(
                    stage,
                    format!(
                        "{label} symbol virtual address 0x{virtual_address:x} is not executable"
                    ),
                )
            })
    }

    fn resolve_symbol_offsets(
        &self,
        data: &[u8],
        required_symbols: &[&str],
        label: &str,
        stage: &'static str,
    ) -> Result<BTreeMap<String, usize>, LoaderError> {
        let sections = parse_sections(data)?;
        let mut offsets = BTreeMap::new();
        for section in &sections {
            if section.section_type != ELF_SECTION_DYNSYM
                && section.section_type != ELF_SECTION_SYMTAB
            {
                continue;
            }
            read_symbol_table(data, &sections, section, required_symbols, &mut offsets)?;
        }
        required_symbols
            .iter()
            .map(|symbol| {
                let virtual_address = offsets.get(*symbol).copied().ok_or_else(|| {
                    LoaderError::new(stage, format!("missing {label} symbol {symbol}"))
                })?;
                self.executable_file_offset(virtual_address, stage, label)
                    .map(|offset| ((*symbol).to_string(), offset))
            })
            .collect()
    }
}

struct LoadSegment {
    file_offset: u64,
    virtual_address: u64,
    file_size: u64,
    executable: bool,
}

impl LoadSegment {
    fn file_offset_for(&self, virtual_address: u64) -> Option<usize> {
        if !self.executable || virtual_address < self.virtual_address {
            return None;
        }
        let relative = virtual_address - self.virtual_address;
        if relative >= self.file_size {
            return None;
        }
        self.file_offset
            .checked_add(relative)
            .and_then(|offset| usize::try_from(offset).ok())
    }
}

struct ElfSection {
    section_type: u32,
    file_offset: u64,
    size: u64,
    link: u32,
    entry_size: u64,
}

fn validate_header(data: &[u8]) -> Result<(), LoaderError> {
    if data.len() < ELF64_HEADER_SIZE || &data[0..ELF_MAGIC.len()] != ELF_MAGIC {
        return Err(LoaderError::new(
            "payload_tls_binary_path",
            "target executable is not an ELF file",
        ));
    }
    if data[ELF_CLASS_OFFSET] != ELF_CLASS_64 || data[ELF_DATA_OFFSET] != ELF_DATA_LITTLE_ENDIAN {
        return Err(LoaderError::new(
            "payload_tls_binary_path",
            "target executable must be ELF64 little-endian",
        ));
    }
    Ok(())
}

fn parse_sections(data: &[u8]) -> Result<Vec<ElfSection>, LoaderError> {
    let table_offset = read_u64(data, ELF_SECTION_HEADER_TABLE_OFFSET_FIELD)?;
    let entry_size = read_u16(data, ELF_SECTION_HEADER_TABLE_ENTRY_SIZE_FIELD)? as u64;
    let count = read_u16(data, ELF_SECTION_HEADER_COUNT_FIELD)? as u64;
    if table_offset == 0 || entry_size == 0 || count == 0 {
        return Err(LoaderError::new(
            "payload_tls_binary_path",
            "target executable has no ELF section table",
        ));
    }
    let mut sections = Vec::new();
    for index in 0..count {
        let offset = checked_table_offset(
            table_offset,
            entry_size,
            index,
            "ELF section-header overflow",
        )?;
        let header = bounded(data, offset, entry_size)?;
        sections.push(ElfSection {
            section_type: read_u32(header, ELF_SECTION_HEADER_TYPE_FIELD)?,
            file_offset: read_u64(header, ELF_SECTION_HEADER_FILE_OFFSET_FIELD)?,
            size: read_u64(header, ELF_SECTION_HEADER_SIZE_FIELD)?,
            link: read_u32(header, ELF_SECTION_HEADER_LINK_FIELD)?,
            entry_size: read_u64(header, ELF_SECTION_ENTRY_SIZE_FIELD)?,
        });
    }
    Ok(sections)
}

fn read_symbol_table(
    data: &[u8],
    sections: &[ElfSection],
    section: &ElfSection,
    required_symbols: &[&str],
    offsets: &mut BTreeMap<String, u64>,
) -> Result<(), LoaderError> {
    if section.entry_size == 0 {
        return Err(LoaderError::new(
            "payload_tls_binary_path",
            "ELF symbol table has zero entry size",
        ));
    }
    let strings = sections
        .get(section.link as usize)
        .ok_or_else(|| LoaderError::new("payload_tls_binary_path", "ELF string table missing"))?;
    let strings = bounded(data, strings.file_offset, strings.size)?;
    let symbol_table = bounded(data, section.file_offset, section.size)?;
    let entry_size = usize::try_from(section.entry_size)
        .map_err(|_| LoaderError::new("payload_tls_binary_path", "ELF symbol entry overflow"))?;
    if entry_size < ELF_SYMBOL_TABLE_ENTRY_SIZE {
        return Err(LoaderError::new(
            "payload_tls_binary_path",
            "ELF symbol table entry is too small",
        ));
    }
    for raw_symbol in symbol_table.chunks_exact(entry_size) {
        let section_index = read_u16(raw_symbol, ELF_SYMBOL_SECTION_INDEX_FIELD)?;
        let virtual_address = read_u64(raw_symbol, ELF_SYMBOL_VALUE_FIELD)?;
        if section_index == ELF_SECTION_UNDEFINED || virtual_address == 0 {
            continue;
        }
        let name_offset = read_u32(raw_symbol, ELF_SYMBOL_NAME_FIELD)?;
        let Some(name) = string_at(strings, name_offset)? else {
            continue;
        };
        if required_symbols.iter().any(|symbol| *symbol == name) {
            offsets.entry(name.to_string()).or_insert(virtual_address);
        }
    }
    Ok(())
}

fn string_at(data: &[u8], offset: u32) -> Result<Option<&str>, LoaderError> {
    let offset = usize::try_from(offset)
        .map_err(|_| LoaderError::new("payload_tls_binary_path", "ELF string offset overflow"))?;
    if offset == 0 {
        return Ok(None);
    }
    let Some(rest) = data.get(offset..) else {
        return Err(LoaderError::new(
            "payload_tls_binary_path",
            "ELF string offset is out of range",
        ));
    };
    let end = rest
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| LoaderError::new("payload_tls_binary_path", "ELF string is unterminated"))?;
    let value = std::str::from_utf8(&rest[..end])
        .map_err(|error| LoaderError::new("payload_tls_binary_path", error.to_string()))?;
    Ok(Some(value))
}

fn parse_build_id_note(data: &[u8]) -> Result<Option<String>, LoaderError> {
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

fn align_note_offset(offset: usize, size: usize) -> Result<usize, LoaderError> {
    offset
        .checked_add(size)
        .and_then(|value| value.checked_add(ELF_NOTE_ALIGNMENT - 1))
        .map(|value| value & !(ELF_NOTE_ALIGNMENT - 1))
        .ok_or_else(|| LoaderError::new("payload_tls_binary_path", "ELF note overflow"))
}

fn checked_table_offset(
    table_offset: u64,
    entry_size: u64,
    index: u64,
    message: &'static str,
) -> Result<u64, LoaderError> {
    table_offset
        .checked_add(
            index
                .checked_mul(entry_size)
                .ok_or_else(|| LoaderError::new("payload_tls_binary_path", message))?,
        )
        .ok_or_else(|| LoaderError::new("payload_tls_binary_path", message))
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn bounded(data: &[u8], offset: u64, size: u64) -> Result<&[u8], LoaderError> {
    let offset = usize::try_from(offset)
        .map_err(|_| LoaderError::new("payload_tls_binary_path", "ELF offset overflow"))?;
    let size = usize::try_from(size)
        .map_err(|_| LoaderError::new("payload_tls_binary_path", "ELF size overflow"))?;
    bounded_usize(data, offset, size)
}

fn bounded_usize(data: &[u8], offset: usize, size: usize) -> Result<&[u8], LoaderError> {
    offset
        .checked_add(size)
        .filter(|end| *end <= data.len())
        .map(|end| &data[offset..end])
        .ok_or_else(|| LoaderError::new("payload_tls_binary_path", "ELF data is truncated"))
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, LoaderError> {
    let bytes = bounded_usize(data, offset, std::mem::size_of::<u16>())?;
    Ok(u16::from_le_bytes(bytes.try_into().expect("bounded u16")))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, LoaderError> {
    let bytes = bounded_usize(data, offset, std::mem::size_of::<u32>())?;
    Ok(u32::from_le_bytes(bytes.try_into().expect("bounded u32")))
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, LoaderError> {
    let bytes = bounded_usize(data, offset, std::mem::size_of::<u64>())?;
    Ok(u64::from_le_bytes(bytes.try_into().expect("bounded u64")))
}
