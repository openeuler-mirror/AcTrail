//! Target executable runtime ABI detection for TLS sync injection.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::{SyncError, SyncResult};

const ELF_MAGIC: &[u8; 4] = b"\x7fELF";
const ELF64_HEADER_SIZE: usize = 64;
const ELF_CLASS_64: u8 = 2;
const ELF_DATA_LITTLE_ENDIAN: u8 = 1;
const PT_INTERP: u32 = 3;
const ELF64_PROGRAM_HEADER_SIZE: usize = 56;
const ELF64_E_PHOFF: usize = 32;
const ELF64_E_PHENTSIZE: usize = 54;
const ELF64_E_PHNUM: usize = 56;
const ELF64_PH_TYPE: usize = 0;
const ELF64_PH_OFFSET: usize = 8;
const ELF64_PH_FILESZ: usize = 32;
const SHEBANG_RECURSION_LIMIT: usize = 6;

struct TargetRuntimeFile {
    file: File,
    generation: TargetFileGeneration,
}

#[derive(Eq, PartialEq)]
struct TargetFileGeneration {
    device: u64,
    inode: u64,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl TargetFileGeneration {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.size(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        }
    }
}

impl TargetRuntimeFile {
    fn open(path: &Path) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let generation = TargetFileGeneration::from_metadata(&file.metadata()?);
        Ok(Self { file, generation })
    }

    fn read_prefix(&mut self) -> std::io::Result<Vec<u8>> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut prefix = Vec::with_capacity(ELF64_HEADER_SIZE);
        (&mut self.file)
            .take(ELF64_HEADER_SIZE as u64)
            .read_to_end(&mut prefix)?;
        Ok(prefix)
    }

    fn read_shebang_line(&mut self) -> std::io::Result<Vec<u8>> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut line = Vec::new();
        BufReader::new(&mut self.file).read_until(b'\n', &mut line)?;
        Ok(line)
    }

    fn elf_interpreter(&mut self, header: &[u8]) -> Result<Option<String>, String> {
        validate_elf64(header)?;
        let phoff = read_u64(header, ELF64_E_PHOFF)?;
        let phentsize = usize::from(read_u16(header, ELF64_E_PHENTSIZE)?);
        if phentsize != ELF64_PROGRAM_HEADER_SIZE {
            return Err(format!(
                "ELF64 program header entry size is {phentsize}, expected {ELF64_PROGRAM_HEADER_SIZE}"
            ));
        }
        let phnum = usize::from(read_u16(header, ELF64_E_PHNUM)?);
        for index in 0..phnum {
            let relative = index
                .checked_mul(phentsize)
                .ok_or("program header index overflow")?;
            let relative = u64::try_from(relative)
                .map_err(|_| "program header offset overflow".to_string())?;
            let offset = phoff
                .checked_add(relative)
                .ok_or("program header offset overflow")?;
            let program_header = self.read_exact_range(offset, phentsize)?;
            if read_u32(&program_header, ELF64_PH_TYPE)? != PT_INTERP {
                continue;
            }
            let interp_offset = read_u64(&program_header, ELF64_PH_OFFSET)?;
            let interp_size = usize::try_from(read_u64(&program_header, ELF64_PH_FILESZ)?)
                .map_err(|_| "PT_INTERP size overflow".to_string())?;
            if !(2..=libc::PATH_MAX as usize).contains(&interp_size) {
                return Err(format!(
                    "PT_INTERP size {interp_size} is outside Linux executable bounds"
                ));
            }
            let raw = self.read_exact_range(interp_offset, interp_size)?;
            let raw = raw.strip_suffix(b"\0").unwrap_or(&raw);
            let interpreter = std::str::from_utf8(raw)
                .map(str::to_string)
                .map_err(|error| format!("PT_INTERP is not UTF-8: {error}"));
            self.ensure_unchanged()?;
            return interpreter.map(Some);
        }
        self.ensure_unchanged()?;
        Ok(None)
    }

    fn read_exact_range(&mut self, offset: u64, size: usize) -> Result<Vec<u8>, String> {
        let size_u64 = u64::try_from(size).map_err(|_| "ELF field size overflow".to_string())?;
        let end = offset
            .checked_add(size_u64)
            .ok_or_else(|| "ELF offset overflow".to_string())?;
        if end > self.generation.size {
            return Err("ELF field is out of bounds".to_string());
        }
        let mut data = vec![0; size];
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|error| format!("cannot seek to ELF field: {error}"))?;
        self.file
            .read_exact(&mut data)
            .map_err(|error| format!("cannot read ELF field: {error}"))?;
        Ok(data)
    }

    fn ensure_unchanged(&self) -> Result<(), String> {
        let current = self
            .file
            .metadata()
            .map_err(|error| format!("cannot restat exec target: {error}"))?;
        if TargetFileGeneration::from_metadata(&current) != self.generation {
            return Err("exec target changed during runtime classification".to_string());
        }
        Ok(())
    }
}

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
    pub interpreter: Option<PathBuf>,
    pub libc: Option<LibcFamily>,
}

impl TargetRuntime {
    pub const fn is_static(&self) -> bool {
        self.interpreter.is_none()
    }
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
    let mut target = TargetRuntimeFile::open(path).map_err(|error| {
        SyncError::new(format!(
            "cannot read exec target {}: {error}",
            path.display()
        ))
    })?;
    let prefix = target.read_prefix().map_err(|error| {
        SyncError::new(format!(
            "cannot read exec target {}: {error}",
            path.display()
        ))
    })?;
    if prefix.starts_with(b"#!") {
        let shebang = target.read_shebang_line().map_err(|error| {
            SyncError::new(format!(
                "cannot read exec target {}: {error}",
                path.display()
            ))
        })?;
        target.ensure_unchanged().map_err(|error| {
            SyncError::new(format!(
                "cannot classify exec target {}: {error}",
                path.display()
            ))
        })?;
        let interpreter = shebang_interpreter(&shebang, path_value)?.ok_or_else(|| {
            SyncError::new(format!(
                "cannot classify exec target {}: invalid shebang",
                path.display()
            ))
        })?;
        return target_runtime_for_path_inner(&interpreter, path_value, depth + 1);
    }
    let interpreter = target.elf_interpreter(&prefix).map_err(|error| {
        SyncError::new(format!(
            "cannot classify exec target {}: {error}",
            path.display()
        ))
    })?;
    let Some(interpreter) = interpreter else {
        return Ok(TargetRuntime {
            path: path.to_path_buf(),
            interpreter: None,
            libc: None,
        });
    };
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
        interpreter: Some(interpreter),
        libc: Some(libc),
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

fn validate_elf64(data: &[u8]) -> Result<(), String> {
    if data.len() < ELF64_HEADER_SIZE || &data[..4] != ELF_MAGIC {
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
