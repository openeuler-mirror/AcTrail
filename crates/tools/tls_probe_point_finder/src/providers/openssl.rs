//! OpenSSL probe-point provider metadata.

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::elf::{DynamicInfo, ElfImage};
use crate::{ToolError, ToolResult};

pub(crate) const NAME: &str = "openssl";
pub(crate) const LIBRARY: &str = "openssl";
pub(crate) const RESOLVER: &str = "openssl-symbols";
pub(crate) const SYMBOLS: &[&str] = &["SSL_read", "SSL_write", "SSL_read_ex", "SSL_write_ex"];

const CONFIDENCE_USER_SPECIFIED: &str = "user-specified";
const CONFIDENCE_DIRECT_NEEDED: &str = "direct-needed";
const CONFIDENCE_PYTHON_SSL_NEEDED: &str = "python-_ssl-needed";
const CONFIDENCE_TRANSITIVE_NEEDED: &str = "transitive-needed";
const CONFIDENCE_RANK_USER_SPECIFIED: u8 = 3;
const CONFIDENCE_RANK_DIRECT_NEEDED: u8 = 2;
const CONFIDENCE_RANK_TRANSITIVE_NEEDED: u8 = 1;
const CONFIDENCE_RANK_UNKNOWN: u8 = 0;
const PYTHON_SSL_EXTENSION_QUERY_ARGS: &[&str] = &["-S", "-c", "import _ssl; print(_ssl.__file__)"];

// Documented dependency-resolution directories. These are used only to resolve
// names already present in a DT_NEEDED dependency graph.
const SYSTEM_LIBRARY_DIRS: &[&str] = &[
    "/lib",
    "/lib64",
    "/usr/lib",
    "/usr/lib64",
    "/lib/x86_64-linux-gnu",
    "/usr/lib/x86_64-linux-gnu",
    "/lib/aarch64-linux-gnu",
    "/usr/lib/aarch64-linux-gnu",
];
const ORIGIN_TOKEN: &str = "$ORIGIN";

#[derive(Clone)]
pub(crate) struct LibraryCandidate {
    pub(crate) path: PathBuf,
    pub(crate) confidence: &'static str,
    pub(crate) counts_as_match: bool,
    pub(crate) note: Option<String>,
}

pub(crate) struct LibrarySearch {
    pub(crate) candidates: Vec<LibraryCandidate>,
    pub(crate) notices: Vec<String>,
}

pub(crate) fn library_candidates(
    image: &ElfImage,
    libraries: &[PathBuf],
    library_search_dirs: &[PathBuf],
) -> ToolResult<LibrarySearch> {
    let mut candidates = BTreeMap::<PathBuf, LibraryCandidate>::new();
    let mut notices = Vec::new();
    for path in libraries {
        insert_candidate(&mut candidates, path, CONFIDENCE_USER_SPECIFIED, true, None)?;
    }
    collect_direct_libssl(image, library_search_dirs, &mut candidates, &mut notices)?;
    collect_python_ssl_libssl(image, library_search_dirs, &mut candidates, &mut notices)?;
    collect_needed_libssl(image, library_search_dirs, &mut candidates, &mut notices)?;
    Ok(LibrarySearch {
        candidates: candidates.into_values().collect(),
        notices,
    })
}

pub(crate) fn direct_library_candidates(
    image: &ElfImage,
    libraries: &[PathBuf],
    library_search_dirs: &[PathBuf],
) -> ToolResult<LibrarySearch> {
    let mut candidates = BTreeMap::<PathBuf, LibraryCandidate>::new();
    let mut notices = Vec::new();
    for path in libraries {
        insert_candidate(&mut candidates, path, CONFIDENCE_USER_SPECIFIED, true, None)?;
    }
    collect_direct_libssl(image, library_search_dirs, &mut candidates, &mut notices)?;
    collect_python_ssl_libssl(image, library_search_dirs, &mut candidates, &mut notices)?;
    Ok(LibrarySearch {
        candidates: candidates.into_values().collect(),
        notices,
    })
}

fn insert_candidate(
    candidates: &mut BTreeMap<PathBuf, LibraryCandidate>,
    path: &Path,
    confidence: &'static str,
    counts_as_match: bool,
    note: Option<String>,
) -> ToolResult<()> {
    if !path.exists() {
        return Err(ToolError::new(format!(
            "shared library path does not exist: {}",
            path.display()
        )));
    }
    let canonical = fs::canonicalize(path)
        .map_err(|error| ToolError::new(format!("cannot resolve {}: {error}", path.display())))?;
    let candidate = LibraryCandidate {
        path: canonical.clone(),
        confidence,
        counts_as_match,
        note,
    };
    if let Some(existing) = candidates.get(&canonical) {
        if candidate_rank(existing.confidence) >= candidate_rank(confidence) {
            return Ok(());
        }
    }
    candidates.insert(canonical, candidate);
    Ok(())
}

fn candidate_rank(confidence: &str) -> u8 {
    match confidence {
        CONFIDENCE_USER_SPECIFIED => CONFIDENCE_RANK_USER_SPECIFIED,
        CONFIDENCE_DIRECT_NEEDED | CONFIDENCE_PYTHON_SSL_NEEDED => CONFIDENCE_RANK_DIRECT_NEEDED,
        CONFIDENCE_TRANSITIVE_NEEDED => CONFIDENCE_RANK_TRANSITIVE_NEEDED,
        _ => CONFIDENCE_RANK_UNKNOWN,
    }
}

fn collect_needed_libssl(
    image: &ElfImage,
    library_search_dirs: &[PathBuf],
    candidates: &mut BTreeMap<PathBuf, LibraryCandidate>,
    notices: &mut Vec<String>,
) -> ToolResult<()> {
    let dynamic = image.dynamic_info()?;
    let origin = image.path().parent().unwrap_or_else(|| Path::new("."));
    let root_dirs = dependency_search_dirs(&dynamic, origin, library_search_dirs);
    let root_label = image
        .path()
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("target")
        .to_string();
    let mut pending = VecDeque::new();
    for needed in &dynamic.needed {
        pending.push_back(NeededEdge {
            name: needed.clone(),
            search_dirs: root_dirs.clone(),
            chain: vec![root_label.clone()],
            depth: DependencyDepth::Direct,
        });
    }
    let mut visited = BTreeMap::<PathBuf, ()>::new();
    while let Some(edge) = pending.pop_front() {
        let Some(path) = resolve_needed_library(&edge.name, &edge.search_dirs) else {
            notices.push(format!("needed_not_found name={}", edge.name));
            continue;
        };
        let canonical = fs::canonicalize(&path).map_err(|error| {
            ToolError::new(format!("cannot resolve {}: {error}", path.display()))
        })?;
        let chain = chain_with(&edge.chain, &edge.name);
        if is_libssl_name(&edge.name) {
            let confidence = match edge.depth {
                DependencyDepth::Direct => CONFIDENCE_DIRECT_NEEDED,
                DependencyDepth::Transitive => CONFIDENCE_TRANSITIVE_NEEDED,
            };
            insert_candidate(
                candidates,
                &canonical,
                confidence,
                true,
                Some(format!("dependency_chain={}", chain.join(" -> "))),
            )?;
            continue;
        }
        if visited.insert(canonical.clone(), ()).is_some() {
            continue;
        }
        let dependency = ElfImage::parse(&canonical)?;
        let dependency_dynamic = dependency.dynamic_info()?;
        let dependency_origin = dependency.path().parent().unwrap_or_else(|| Path::new("."));
        let dependency_dirs =
            dependency_search_dirs(&dependency_dynamic, dependency_origin, library_search_dirs);
        for needed in &dependency_dynamic.needed {
            pending.push_back(NeededEdge {
                name: needed.clone(),
                search_dirs: dependency_dirs.clone(),
                chain: chain.clone(),
                depth: DependencyDepth::Transitive,
            });
        }
    }
    Ok(())
}

fn collect_direct_libssl(
    image: &ElfImage,
    library_search_dirs: &[PathBuf],
    candidates: &mut BTreeMap<PathBuf, LibraryCandidate>,
    notices: &mut Vec<String>,
) -> ToolResult<()> {
    let root_label = image
        .path()
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("target")
        .to_string();
    collect_direct_libssl_from_root(
        image,
        library_search_dirs,
        candidates,
        notices,
        CONFIDENCE_DIRECT_NEEDED,
        &root_label,
        None,
    )
}

fn collect_python_ssl_libssl(
    image: &ElfImage,
    library_search_dirs: &[PathBuf],
    candidates: &mut BTreeMap<PathBuf, LibraryCandidate>,
    notices: &mut Vec<String>,
) -> ToolResult<()> {
    if !is_python_executable(image.path()) {
        return Ok(());
    }
    let Some(extension_path) = python_ssl_extension_path(image.path(), notices)? else {
        return Ok(());
    };
    let extension = ElfImage::parse(&extension_path)?;
    let root_label = format!(
        "{}:_ssl",
        image
            .path()
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("python")
    );
    collect_direct_libssl_from_root(
        &extension,
        library_search_dirs,
        candidates,
        notices,
        CONFIDENCE_PYTHON_SSL_NEEDED,
        &root_label,
        Some(format!("python_ssl_extension={}", extension_path.display())),
    )
}

fn collect_direct_libssl_from_root(
    image: &ElfImage,
    library_search_dirs: &[PathBuf],
    candidates: &mut BTreeMap<PathBuf, LibraryCandidate>,
    notices: &mut Vec<String>,
    confidence: &'static str,
    root_label: &str,
    extra_note: Option<String>,
) -> ToolResult<()> {
    let dynamic = image.dynamic_info()?;
    let origin = image.path().parent().unwrap_or_else(|| Path::new("."));
    let search_dirs = dependency_search_dirs(&dynamic, origin, library_search_dirs);
    for needed in &dynamic.needed {
        if !is_libssl_name(needed) {
            continue;
        }
        let Some(path) = resolve_needed_library(needed, &search_dirs) else {
            notices.push(format!("needed_not_found name={needed}"));
            continue;
        };
        let canonical = fs::canonicalize(&path).map_err(|error| {
            ToolError::new(format!("cannot resolve {}: {error}", path.display()))
        })?;
        insert_candidate(
            candidates,
            &canonical,
            confidence,
            true,
            Some(direct_libssl_note(
                root_label,
                needed,
                extra_note.as_deref(),
            )),
        )?;
    }
    Ok(())
}

fn direct_libssl_note(root_label: &str, needed: &str, extra: Option<&str>) -> String {
    let mut note = format!("dependency_chain={root_label} -> {needed}");
    if let Some(extra) = extra {
        note.push(' ');
        note.push_str(extra);
    }
    note
}

fn python_ssl_extension_path(
    python: &Path,
    notices: &mut Vec<String>,
) -> ToolResult<Option<PathBuf>> {
    let output = Command::new(python)
        .args(PYTHON_SSL_EXTENSION_QUERY_ARGS)
        .output()
        .map_err(|error| {
            ToolError::new(format!(
                "cannot query Python _ssl extension from {}: {error}",
                python.display()
            ))
        })?;
    if !output.status.success() {
        notices.push(format!(
            "python_ssl_extension_unavailable binary={} status={} stderr={}",
            python.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
        return Ok(None);
    }
    let raw_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw_path.is_empty() {
        notices.push(format!(
            "python_ssl_extension_unavailable binary={} reason=empty_stdout",
            python.display()
        ));
        return Ok(None);
    }
    let path = PathBuf::from(raw_path);
    fs::canonicalize(&path).map(Some).map_err(|error| {
        ToolError::new(format!(
            "cannot resolve Python _ssl extension {}: {error}",
            path.display()
        ))
    })
}

fn is_python_executable(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let Some(rest) = name.strip_prefix("python") else {
        return false;
    };
    rest.is_empty()
        || rest
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
}

struct NeededEdge {
    name: String,
    search_dirs: Vec<PathBuf>,
    chain: Vec<String>,
    depth: DependencyDepth,
}

#[derive(Copy, Clone)]
enum DependencyDepth {
    Direct,
    Transitive,
}

fn dependency_search_dirs(
    dynamic: &DynamicInfo,
    origin: &Path,
    library_search_dirs: &[PathBuf],
) -> Vec<PathBuf> {
    let mut search_dirs = dynamic_dirs(dynamic, origin);
    search_dirs.extend(library_search_dirs.iter().cloned());
    search_dirs.extend(SYSTEM_LIBRARY_DIRS.iter().map(PathBuf::from));
    search_dirs
}

fn chain_with(chain: &[String], name: &str) -> Vec<String> {
    let mut next = chain.to_vec();
    next.push(name.to_string());
    next
}

fn dynamic_dirs(dynamic: &DynamicInfo, origin: &Path) -> Vec<PathBuf> {
    dynamic
        .rpath
        .iter()
        .chain(dynamic.runpath.iter())
        .map(|entry| expand_origin(entry, origin))
        .collect()
}

fn expand_origin(entry: &str, origin: &Path) -> PathBuf {
    if let Some(rest) = entry.strip_prefix(ORIGIN_TOKEN) {
        origin.join(rest.trim_start_matches('/'))
    } else {
        PathBuf::from(entry)
    }
}

fn resolve_needed_library(name: &str, search_dirs: &[PathBuf]) -> Option<PathBuf> {
    let needed = Path::new(name);
    if needed.is_absolute() && needed.exists() {
        return Some(needed.to_path_buf());
    }
    search_dirs
        .iter()
        .map(|directory| directory.join(name))
        .find(|path| path.exists())
}

fn is_libssl_name(name: &str) -> bool {
    Path::new(name)
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|basename| basename.starts_with("libssl.so"))
}
