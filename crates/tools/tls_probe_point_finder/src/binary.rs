//! Resolve command names and launcher scripts to concrete ELF binaries.

use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::{ToolError, ToolResult};

const ELF_MAGIC: &[u8; 4] = b"\x7fELF";
const KNOWN_TLS_LAUNCHER_RUNTIMES: &[&str] = &["node", "bun"];

pub(crate) fn resolve_entry_elf(path: &Path) -> ToolResult<PathBuf> {
    let entry = resolve_command_or_path(path)?;
    if is_elf(&entry) {
        return Ok(entry);
    }
    if let Some(candidate) = non_elf_entry_candidate(&entry)? {
        return Ok(candidate);
    }
    Err(ToolError::new(format!(
        "{} is not an ELF file and no concrete ELF runtime was found",
        entry.display()
    )))
}

fn resolve_command_or_path(path: &Path) -> ToolResult<PathBuf> {
    let raw = path.as_os_str().to_string_lossy();
    if !raw.contains('/') {
        return resolve_from_path(&raw);
    }
    let resolved = fs::canonicalize(path)
        .map_err(|error| ToolError::new(format!("cannot resolve {}: {error}", path.display())))?;
    if !resolved.is_file() {
        return Err(ToolError::new(format!(
            "not a file: {}",
            resolved.display()
        )));
    }
    Ok(resolved)
}

fn resolve_from_path(command: &str) -> ToolResult<PathBuf> {
    let path_var = env::var_os("PATH").ok_or_else(|| ToolError::new("PATH is not set"))?;
    for directory in env::split_paths(&path_var) {
        let candidate = directory.join(command);
        if candidate.is_file() {
            return fs::canonicalize(&candidate).map_err(|error| {
                ToolError::new(format!("cannot resolve {}: {error}", candidate.display()))
            });
        }
    }
    Err(ToolError::new(format!(
        "command not found on PATH: {command}"
    )))
}

fn is_elf(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut magic = [0_u8; ELF_MAGIC.len()];
    file.read_exact(&mut magic).is_ok() && &magic == ELF_MAGIC
}

fn non_elf_entry_candidate(entry: &Path) -> ToolResult<Option<PathBuf>> {
    let sibling = entry
        .parent()
        .ok_or_else(|| ToolError::new(format!("{} has no parent directory", entry.display())))?
        .join(format!(
            ".{}",
            entry
                .file_name()
                .ok_or_else(|| ToolError::new("entry path has no file name"))?
                .to_string_lossy()
        ));
    if sibling.is_file() && is_elf(&sibling) {
        return fs::canonicalize(&sibling).map(Some).map_err(|error| {
            ToolError::new(format!("cannot resolve {}: {error}", sibling.display()))
        });
    }
    resolve_shebang_runtime(entry)
}

fn resolve_shebang_runtime(entry: &Path) -> ToolResult<Option<PathBuf>> {
    let raw = fs::read_to_string(entry).map_err(|error| {
        ToolError::new(format!("cannot read launcher {}: {error}", entry.display()))
    })?;
    let Some(first_line) = raw.lines().next() else {
        return Ok(None);
    };
    let Some(shebang) = first_line.strip_prefix("#!") else {
        return Ok(None);
    };
    let parts = split_shell_words(shebang.trim())?;
    if parts.is_empty() {
        return Ok(None);
    }
    let runtime = shebang_runtime_name(&parts)?;
    if !is_known_tls_launcher_runtime(&runtime) {
        return Ok(None);
    }
    let binary = resolve_from_path(&runtime)?;
    if !is_elf(&binary) {
        return Err(ToolError::new(format!(
            "{} shebang runtime {} is not an ELF file",
            entry.display(),
            binary.display()
        )));
    }
    Ok(Some(binary))
}

fn is_known_tls_launcher_runtime(runtime: &str) -> bool {
    KNOWN_TLS_LAUNCHER_RUNTIMES.contains(&runtime) || is_python_runtime_name(runtime)
}

fn is_python_runtime_name(runtime: &str) -> bool {
    let Some(rest) = runtime.strip_prefix("python") else {
        return false;
    };
    rest.is_empty()
        || rest
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
}

fn shebang_runtime_name(parts: &[String]) -> ToolResult<String> {
    let command = Path::new(&parts[0])
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(parts[0].as_str());
    if command != "env" {
        return Ok(command.to_string());
    }
    if parts.get(1).is_some_and(|part| part == "-S") {
        let rest = parts.get(2..).unwrap_or_default().join(" ");
        let split = split_shell_words(&rest)?;
        return split
            .first()
            .cloned()
            .ok_or_else(|| ToolError::new("env -S shebang is missing command"));
    }
    parts
        .iter()
        .skip(1)
        .find(|part| !part.starts_with('-'))
        .cloned()
        .ok_or_else(|| ToolError::new("env shebang is missing command"))
}

fn split_shell_words(value: &str) -> ToolResult<Vec<String>> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if character == active {
                quote = None;
            } else {
                current.push(character);
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            c if c.is_whitespace() => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if escaped || quote.is_some() {
        return Err(ToolError::new("unterminated escape or quote in shebang"));
    }
    if !current.is_empty() {
        words.push(current);
    }
    Ok(words)
}
