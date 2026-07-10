//! Target executable runtime ABI detection for TLS sync injection.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::{SyncError, SyncResult};

const ELF_MAGIC: &[u8; 4] = b"\x7fELF";
const ELF_CLASS_64: u8 = 2;
const ELF_DATA_LITTLE_ENDIAN: u8 = 1;
const PT_INTERP: u32 = 3;
const ELF64_E_PHOFF: usize = 32;
const ELF64_E_PHENTSIZE: usize = 54;
const ELF64_E_PHNUM: usize = 56;
const ELF64_PH_TYPE: usize = 0;
const ELF64_PH_OFFSET: usize = 8;
const ELF64_PH_FILESZ: usize = 32;
const SHEBANG_RECURSION_LIMIT: usize = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LibcFamily {
    Glibc,
    Musl,
}

impl LibcFamily {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Glibc => "glibc",
            Self::Musl => "musl",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetRuntime {
    pub path: PathBuf,
    pub interpreter: PathBuf,
    pub libc: LibcFamily,
}

pub fn resolve_target_runtime(
    program: &OsStr,
    path_value: Option<&OsStr>,
) -> SyncResult<TargetRuntime> {
    let path = resolve_program_path(program, path_value)?;
    target_runtime_for_path(&path, path_value)
}

pub fn target_runtime_for_path(
    path: &Path,
    path_value: Option<&OsStr>,
) -> SyncResult<TargetRuntime> {
    target_runtime_for_path_inner(path, path_value, 0)
}

pub fn resolve_program_path(program: &OsStr, path_value: Option<&OsStr>) -> SyncResult<PathBuf> {
    if program.is_empty() {
        return Err(SyncError::new("exec target is empty"));
    }
    let raw = Path::new(program);
    if program.as_bytes().contains(&b'/') {
        let candidate = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|error| SyncError::new(format!("resolve cwd: {error}")))?
                .join(raw)
        };
        return canonical_existing_file(&candidate);
    }
    if let Some(candidate) = current_dir_candidate(raw) {
        return Ok(candidate);
    }
    let path_value = path_value
        .map(OsString::from)
        .or_else(|| std::env::var_os("PATH"))
        .ok_or_else(|| SyncError::new("PATH is not set"))?;
    for directory in std::env::split_paths(&path_value) {
        let candidate = directory.join(raw);
        if candidate.is_file() {
            return canonical_existing_file(&candidate);
        }
    }
    Err(SyncError::new(format!(
        "exec target not found on PATH: {}",
        program.to_string_lossy()
    )))
}

fn target_runtime_for_path_inner(
    path: &Path,
    path_value: Option<&OsStr>,
    depth: usize,
) -> SyncResult<TargetRuntime> {
    if depth > SHEBANG_RECURSION_LIMIT {
        return Err(SyncError::new(format!(
            "script interpreter recursion limit exceeded for {}",
            path.display()
        )));
    }
    let data = std::fs::read(path).map_err(|error| {
        SyncError::new(format!(
            "cannot read exec target {}: {error}",
            path.display()
        ))
    })?;
    if let Some(interpreter) = shebang_interpreter(&data, path_value)? {
        return target_runtime_for_path_inner(&interpreter, path_value, depth + 1);
    }
    let interpreter = elf_interpreter(&data).map_err(|error| {
        SyncError::new(format!(
            "cannot classify exec target {}: {error}",
            path.display()
        ))
    })?;
    let interpreter = PathBuf::from(interpreter);
    let libc = libc_family_for_interpreter(&interpreter).ok_or_else(|| {
        SyncError::new(format!(
            "unsupported ELF interpreter {} for exec target {}",
            interpreter.display(),
            path.display()
        ))
    })?;
    Ok(TargetRuntime {
        path: path.to_path_buf(),
        interpreter,
        libc,
    })
}

fn canonical_existing_file(path: &Path) -> SyncResult<PathBuf> {
    if !path.is_file() {
        return Err(SyncError::new(format!(
            "exec target is not a file: {}",
            path.display()
        )));
    }
    std::fs::canonicalize(path).map_err(|error| {
        SyncError::new(format!(
            "cannot resolve exec target {}: {error}",
            path.display()
        ))
    })
}

fn current_dir_candidate(path: &Path) -> Option<PathBuf> {
    let candidate = std::env::current_dir().ok()?.join(path);
    candidate
        .is_file()
        .then(|| canonical_existing_file(&candidate).ok())
        .flatten()
}

fn shebang_interpreter(data: &[u8], path_value: Option<&OsStr>) -> SyncResult<Option<PathBuf>> {
    if !data.starts_with(b"#!") {
        return Ok(None);
    }
    let end = data
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(data.len());
    let line = std::str::from_utf8(&data[2..end])
        .map_err(|error| SyncError::new(format!("script shebang is not UTF-8: {error}")))?
        .trim();
    let mut tokens = line.split_whitespace();
    let Some(interpreter) = tokens.next() else {
        return Err(SyncError::new("script shebang has no interpreter"));
    };
    if Path::new(interpreter)
        .file_name()
        .is_some_and(|name| name == OsStr::new("env"))
    {
        let env_target = env_shebang_target(tokens.collect::<Vec<_>>())?;
        return resolve_program_path(OsStr::new(env_target), path_value).map(Some);
    }
    let interpreter = Path::new(interpreter);
    if interpreter.is_absolute() {
        canonical_existing_file(interpreter).map(Some)
    } else {
        resolve_program_path(interpreter.as_os_str(), path_value).map(Some)
    }
}

fn env_shebang_target(tokens: Vec<&str>) -> SyncResult<&str> {
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index];
        if token == "-S" {
            index += 1;
            continue;
        }
        if token.starts_with('-') {
            index += 1;
            continue;
        }
        if token.contains('=') {
            index += 1;
            continue;
        }
        return Ok(token);
    }
    Err(SyncError::new("/usr/bin/env shebang has no target command"))
}

fn elf_interpreter(data: &[u8]) -> Result<String, String> {
    validate_elf64(data)?;
    let phoff = read_u64(data, ELF64_E_PHOFF)?;
    let phentsize = usize::from(read_u16(data, ELF64_E_PHENTSIZE)?);
    let phnum = usize::from(read_u16(data, ELF64_E_PHNUM)?);
    let phoff = usize::try_from(phoff).map_err(|_| "program header offset overflow".to_string())?;
    for index in 0..phnum {
        let offset = phoff
            .checked_add(
                index
                    .checked_mul(phentsize)
                    .ok_or("program header index overflow")?,
            )
            .ok_or("program header offset overflow")?;
        let header = bounded(data, offset, phentsize)?;
        if read_u32(header, ELF64_PH_TYPE)? != PT_INTERP {
            continue;
        }
        let interp_offset = usize::try_from(read_u64(header, ELF64_PH_OFFSET)?)
            .map_err(|_| "PT_INTERP offset overflow".to_string())?;
        let interp_size = usize::try_from(read_u64(header, ELF64_PH_FILESZ)?)
            .map_err(|_| "PT_INTERP size overflow".to_string())?;
        let raw = bounded(data, interp_offset, interp_size)?;
        let raw = raw.strip_suffix(b"\0").unwrap_or(raw);
        return std::str::from_utf8(raw)
            .map(str::to_string)
            .map_err(|error| format!("PT_INTERP is not UTF-8: {error}"));
    }
    Err("ELF has no PT_INTERP".to_string())
}

fn validate_elf64(data: &[u8]) -> Result<(), String> {
    if data.len() < 64 || &data[..4] != ELF_MAGIC {
        return Err("not an ELF executable and not a script".to_string());
    }
    if data[4] != ELF_CLASS_64 {
        return Err(
            "only ELF64 exec targets are supported for TLS sync runtime selection".to_string(),
        );
    }
    if data[5] != ELF_DATA_LITTLE_ENDIAN {
        return Err(
            "only little-endian ELF exec targets are supported for TLS sync runtime selection"
                .to_string(),
        );
    }
    Ok(())
}

fn libc_family_for_interpreter(interpreter: &Path) -> Option<LibcFamily> {
    let text = interpreter.as_os_str().to_string_lossy();
    if text.contains("ld-musl") {
        return Some(LibcFamily::Musl);
    }
    if text.contains("ld-linux") {
        return Some(LibcFamily::Glibc);
    }
    None
}

fn bounded(data: &[u8], offset: usize, size: usize) -> Result<&[u8], String> {
    let end = offset
        .checked_add(size)
        .ok_or_else(|| "ELF offset overflow".to_string())?;
    data.get(offset..end)
        .ok_or_else(|| "ELF field is out of bounds".to_string())
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, String> {
    let bytes = bounded(data, offset, 2)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, String> {
    let bytes = bounded(data, offset, 4)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, String> {
    let bytes = bounded(data, offset, 8)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}
