//! ELF constants used by the minimal ELF64 little-endian parser.

pub(super) const ELF64_HEADER_SIZE: usize = 64;
pub(super) const ELF_MAGIC: &[u8; 4] = b"\x7fELF";
pub(super) const ELF_CLASS_OFFSET: usize = 4;
pub(super) const ELF_DATA_OFFSET: usize = 5;
pub(super) const ELF_CLASS_64: u8 = 2;
pub(super) const ELF_DATA_LITTLE_ENDIAN: u8 = 1;
pub(super) const ELF_MACHINE_FIELD: usize = 18;
pub(super) const ELF_MACHINE_X86_64: u16 = 62;
pub(super) const ELF_MACHINE_AARCH64: u16 = 183;
pub(super) const ELF_PROGRAM_HEADER_TABLE_OFFSET_FIELD: usize = 32;
pub(super) const ELF_SECTION_HEADER_TABLE_OFFSET_FIELD: usize = 40;
pub(super) const ELF_PROGRAM_HEADER_ENTRY_SIZE_FIELD: usize = 54;
pub(super) const ELF_PROGRAM_HEADER_COUNT_FIELD: usize = 56;
pub(super) const ELF_SECTION_HEADER_TABLE_ENTRY_SIZE_FIELD: usize = 58;
pub(super) const ELF_SECTION_HEADER_COUNT_FIELD: usize = 60;
pub(super) const ELF_SECTION_NAME_TABLE_INDEX_FIELD: usize = 62;

pub(super) const ELF_PROGRAM_HEADER_TYPE_FIELD: usize = 0;
pub(super) const ELF_PROGRAM_HEADER_FILE_OFFSET_FIELD: usize = 8;
pub(super) const ELF_PROGRAM_HEADER_VADDR_FIELD: usize = 16;
pub(super) const ELF_PROGRAM_HEADER_FILE_SIZE_FIELD: usize = 32;
pub(super) const ELF_PROGRAM_HEADER_LOAD: u32 = 1;
pub(super) const ELF_PROGRAM_HEADER_DYNAMIC: u32 = 2;
pub(super) const ELF_PROGRAM_HEADER_NOTE: u32 = 4;

pub(super) const ELF_SECTION_HEADER_NAME_FIELD: usize = 0;
pub(super) const ELF_SECTION_HEADER_TYPE_FIELD: usize = 4;
pub(super) const ELF_SECTION_HEADER_FILE_OFFSET_FIELD: usize = 24;
pub(super) const ELF_SECTION_HEADER_SIZE_FIELD: usize = 32;
pub(super) const ELF_SECTION_HEADER_LINK_FIELD: usize = 40;
pub(super) const ELF_SECTION_ENTRY_SIZE_FIELD: usize = 56;
pub(super) const ELF_SECTION_SYMTAB: u32 = 2;
pub(super) const ELF_SECTION_NOTE: u32 = 7;
pub(super) const ELF_SECTION_DYNSYM: u32 = 11;

pub(super) const ELF_SYMBOL_NAME_FIELD: usize = 0;
pub(super) const ELF_SYMBOL_INFO_FIELD: usize = 4;
pub(super) const ELF_SYMBOL_SECTION_INDEX_FIELD: usize = 6;
pub(super) const ELF_SYMBOL_VALUE_FIELD: usize = 8;
pub(super) const ELF_SYMBOL_SIZE_FIELD: usize = 16;
pub(super) const ELF_SYMBOL_TABLE_ENTRY_SIZE: usize = 24;
pub(super) const ELF_SYMBOL_TYPE_MASK: u8 = 0x0f;
pub(super) const ELF_SYMBOL_TYPE_FUNC: u8 = 2;
pub(super) const ELF_SYMBOL_BIND_LOCAL: u8 = 0;
pub(super) const ELF_SYMBOL_BIND_GLOBAL: u8 = 1;
pub(super) const ELF_SYMBOL_BIND_WEAK: u8 = 2;
pub(super) const ELF_SYMBOL_BIND_LOOS: u8 = 10;
pub(super) const ELF_SYMBOL_BIND_HIOS: u8 = 12;
pub(super) const ELF_SYMBOL_BIND_LOPROC: u8 = 13;
pub(super) const ELF_SYMBOL_BIND_HIPROC: u8 = 15;
pub(super) const ELF_SECTION_UNDEFINED: u16 = 0;

pub(super) const ELF_NOTE_HEADER_SIZE: usize = 12;
pub(super) const ELF_NOTE_NAME_SIZE_FIELD: usize = 0;
pub(super) const ELF_NOTE_DESCRIPTION_SIZE_FIELD: usize = 4;
pub(super) const ELF_NOTE_TYPE_FIELD: usize = 8;
pub(super) const ELF_NOTE_ALIGNMENT: usize = 4;
pub(super) const ELF_NOTE_GNU_BUILD_ID: u32 = 3;
pub(super) const ELF_NOTE_NAME_GNU: &[u8] = b"GNU\0";

pub(super) const ELF_DYNAMIC_TAG_FIELD: usize = 0;
pub(super) const ELF_DYNAMIC_VALUE_FIELD: usize = 8;
pub(super) const ELF_DYNAMIC_ENTRY_SIZE: usize = 16;
pub(super) const ELF_DYNAMIC_NULL: i64 = 0;
pub(super) const ELF_DYNAMIC_NEEDED: i64 = 1;
pub(super) const ELF_DYNAMIC_STRTAB: i64 = 5;
pub(super) const ELF_DYNAMIC_STRSZ: i64 = 10;
pub(super) const ELF_DYNAMIC_RPATH: i64 = 15;
pub(super) const ELF_DYNAMIC_RUNPATH: i64 = 29;
