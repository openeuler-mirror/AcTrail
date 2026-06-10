//! Go crypto/tls probe-point provider metadata.

use std::collections::BTreeMap;

use crate::elf::ElfImage;
use crate::{ToolError, ToolResult};

pub(crate) const NAME: &str = "go";
pub(crate) const LIBRARY: &str = "go";
pub(crate) const RESOLVER: &str = "go-pclntab";
pub(crate) const WRITE_SYMBOL: &str = "crypto/tls.(*Conn).Write";
pub(crate) const READ_SYMBOL: &str = "crypto/tls.(*Conn).Read";
pub(crate) const RUNTIME_MEMMOVE_SYMBOL: &str = "runtime.memmove";
pub(crate) const SYMBOLS: &[&str] = &[WRITE_SYMBOL, READ_SYMBOL, RUNTIME_MEMMOVE_SYMBOL];

const GOPCLNTAB_SECTION: &str = ".gopclntab";
const PCLNTAB_MAGIC: u32 = 0xfffffff1;
const PTR_SIZE_OFFSET: usize = 7;
const NFUNC_OFFSET: usize = 8;
const TEXT_START_OFFSET: usize = 24;
const FUNCNAME_OFFSET_OFFSET: usize = 32;
const FUNCTAB_OFFSET_OFFSET: usize = 64;
const FUNCTAB_ENTRY_SIZE: usize = 8;
const FUNC_NAME_OFF_FIELD: usize = 4;

pub(crate) fn resolve_pclntab_symbols(
    image: &ElfImage,
    required_symbols: &[&str],
) -> ToolResult<Option<BTreeMap<String, u64>>> {
    let Some(section) = image.section_data(GOPCLNTAB_SECTION)? else {
        return Ok(None);
    };
    let pclntab = GoPclntab::parse(section, image.section_virtual_address(".text"))?;
    let symbols = pclntab.find_symbols(required_symbols)?;
    if required_symbols
        .iter()
        .all(|symbol| symbols.contains_key(*symbol))
    {
        Ok(Some(symbols))
    } else {
        Ok(None)
    }
}

struct GoPclntab<'a> {
    data: &'a [u8],
    nfunc: usize,
    text_start: u64,
    funcname_offset: usize,
    functab_offset: usize,
}

impl<'a> GoPclntab<'a> {
    fn parse(data: &'a [u8], text_section_address: Option<u64>) -> ToolResult<Self> {
        let magic = read_u32(data, 0)?;
        if magic != PCLNTAB_MAGIC {
            return Err(ToolError::new(format!(
                "unsupported Go pclntab magic 0x{magic:x}"
            )));
        }
        let ptr_size = read_u8(data, PTR_SIZE_OFFSET)?;
        if ptr_size != std::mem::size_of::<u64>() as u8 {
            return Err(ToolError::new(format!(
                "unsupported Go pclntab pointer size {ptr_size}"
            )));
        }
        let mut text_start = read_u64(data, TEXT_START_OFFSET)?;
        if text_start == 0 {
            text_start = text_section_address.ok_or_else(|| {
                ToolError::new("Go pclntab has zero textStart and no .text section")
            })?;
        }
        Ok(Self {
            data,
            nfunc: checked_usize(read_u64(data, NFUNC_OFFSET)?, "Go nfunc")?,
            text_start,
            funcname_offset: checked_usize(
                read_u64(data, FUNCNAME_OFFSET_OFFSET)?,
                "Go funcname offset",
            )?,
            functab_offset: checked_usize(
                read_u64(data, FUNCTAB_OFFSET_OFFSET)?,
                "Go functab offset",
            )?,
        })
    }

    fn find_symbols(&self, required: &[&str]) -> ToolResult<BTreeMap<String, u64>> {
        let mut found = BTreeMap::new();
        for index in 0..self.nfunc {
            let entry_offset = checked_add(
                self.functab_offset,
                checked_mul(index, FUNCTAB_ENTRY_SIZE, "Go functab overflow")?,
                "Go functab overflow",
            )?;
            let entry_off = read_u32(self.data, entry_offset)? as u64;
            let func_offset = checked_add(
                self.functab_offset,
                read_u32(self.data, entry_offset + std::mem::size_of::<u32>())? as usize,
                "Go func offset overflow",
            )?;
            let name_offset = read_i32(self.data, func_offset + FUNC_NAME_OFF_FIELD)?;
            if name_offset < 0 {
                continue;
            }
            let name_offset = checked_add(
                self.funcname_offset,
                name_offset as usize,
                "Go func name offset overflow",
            )?;
            let name = string_at(self.data, name_offset)?;
            if required.contains(&name) {
                found.insert(
                    name.to_string(),
                    self.text_start
                        .checked_add(entry_off)
                        .ok_or_else(|| ToolError::new("Go function address overflow"))?,
                );
            }
            if found.len() == required.len() {
                break;
            }
        }
        Ok(found)
    }
}

fn checked_usize(value: u64, label: &str) -> ToolResult<usize> {
    usize::try_from(value).map_err(|_| ToolError::new(format!("{label} overflows usize")))
}

fn checked_add(left: usize, right: usize, message: &'static str) -> ToolResult<usize> {
    left.checked_add(right)
        .ok_or_else(|| ToolError::new(message))
}

fn checked_mul(left: usize, right: usize, message: &'static str) -> ToolResult<usize> {
    left.checked_mul(right)
        .ok_or_else(|| ToolError::new(message))
}

fn read_u8(data: &[u8], offset: usize) -> ToolResult<u8> {
    bounded(data, offset, std::mem::size_of::<u8>()).map(|bytes| bytes[0])
}

fn read_u32(data: &[u8], offset: usize) -> ToolResult<u32> {
    let bytes = bounded(data, offset, std::mem::size_of::<u32>())?;
    Ok(u32::from_le_bytes(bytes.try_into().expect("bounded u32")))
}

fn read_i32(data: &[u8], offset: usize) -> ToolResult<i32> {
    let bytes = bounded(data, offset, std::mem::size_of::<i32>())?;
    Ok(i32::from_le_bytes(bytes.try_into().expect("bounded i32")))
}

fn read_u64(data: &[u8], offset: usize) -> ToolResult<u64> {
    let bytes = bounded(data, offset, std::mem::size_of::<u64>())?;
    Ok(u64::from_le_bytes(bytes.try_into().expect("bounded u64")))
}

fn bounded(data: &[u8], offset: usize, size: usize) -> ToolResult<&[u8]> {
    offset
        .checked_add(size)
        .filter(|end| *end <= data.len())
        .map(|end| &data[offset..end])
        .ok_or_else(|| ToolError::new("Go pclntab is truncated"))
}

fn string_at(data: &[u8], offset: usize) -> ToolResult<&str> {
    let rest = data
        .get(offset..)
        .ok_or_else(|| ToolError::new("Go function name offset is out of range"))?;
    let end = rest
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| ToolError::new("Go function name is unterminated"))?;
    std::str::from_utf8(&rest[..end]).map_err(|error| ToolError::new(error.to_string()))
}
