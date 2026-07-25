use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::elf::{DynamicInfo, ElfImage};
use crate::probe_detector::contract::detector::{DetectorConfigError, ProbeDetectorConfig};
use crate::{ToolError, ToolResult};

use super::OpenSslSharedLibraryDiscoveryProbeDetectorConfig;

const CONFIDENCE_USER_SPECIFIED: &str = "user-specified";
const CONFIDENCE_DIRECT_NEEDED: &str = "direct-needed";
const CONFIDENCE_PYTHON_SSL_NEEDED: &str = "python-_ssl-needed";
const CONFIDENCE_TRANSITIVE_NEEDED: &str = "transitive-needed";
const PYTHON_SSL_EXTENSION_QUERY_ARGS: &[&str] = &["-S", "-c", "import _ssl; print(_ssl.__file__)"];
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
    pub(crate) note: Option<String>,
}

pub(crate) struct LibrarySearch {
    pub(crate) candidates: Vec<LibraryCandidate>,
    pub(crate) notices: Vec<String>,
}

pub(crate) struct OpenSslSharedLibraryDiscoveryProbeDetector {
    config: OpenSslSharedLibraryDiscoveryProbeDetectorConfig,
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

impl OpenSslSharedLibraryDiscoveryProbeDetector {
    pub(crate) fn try_new(
        config: OpenSslSharedLibraryDiscoveryProbeDetectorConfig,
    ) -> Result<Self, DetectorConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    pub(crate) fn discover(
        &self,
        image: &ElfImage,
        libraries: &[PathBuf],
        library_search_dirs: &[PathBuf],
        include_transitive: bool,
    ) -> ToolResult<LibrarySearch> {
        let mut candidates = BTreeMap::<PathBuf, LibraryCandidate>::new();
        let mut notices = Vec::new();
        for path in libraries {
            self.insert_candidate(&mut candidates, path, CONFIDENCE_USER_SPECIFIED, None)?;
        }
        self.collect_direct_libssl(image, library_search_dirs, &mut candidates, &mut notices)?;
        if self.config.python_ssl_query_enabled {
            self.collect_python_ssl_libssl(
                image,
                library_search_dirs,
                &mut candidates,
                &mut notices,
            )?;
        }
        if include_transitive {
            self.collect_needed_libssl(image, library_search_dirs, &mut candidates, &mut notices)?;
        }
        Ok(LibrarySearch {
            candidates: candidates.into_values().collect(),
            notices,
        })
    }

    fn insert_candidate(
        &self,
        candidates: &mut BTreeMap<PathBuf, LibraryCandidate>,
        path: &Path,
        confidence: &'static str,
        note: Option<String>,
    ) -> ToolResult<()> {
        if !path.exists() {
            return Err(ToolError::new(format!(
                "shared library path does not exist: {}",
                path.display()
            )));
        }
        let canonical = fs::canonicalize(path).map_err(|error| {
            ToolError::new(format!("cannot resolve {}: {error}", path.display()))
        })?;
        let candidate = LibraryCandidate {
            path: canonical.clone(),
            confidence,
            note,
        };
        if candidates
            .get(&canonical)
            .is_some_and(|existing| Self::rank(existing.confidence) >= Self::rank(confidence))
        {
            return Ok(());
        }
        candidates.insert(canonical, candidate);
        Ok(())
    }

    fn collect_needed_libssl(
        &self,
        image: &ElfImage,
        library_search_dirs: &[PathBuf],
        candidates: &mut BTreeMap<PathBuf, LibraryCandidate>,
        notices: &mut Vec<String>,
    ) -> ToolResult<()> {
        let dynamic = image.dynamic_info()?;
        let origin = image.path().parent().unwrap_or_else(|| Path::new("."));
        let root_dirs = Self::dependency_search_dirs(&dynamic, origin, library_search_dirs);
        let root_label = Self::file_label(image.path(), "target");
        let mut pending = dynamic
            .needed
            .iter()
            .map(|name| NeededEdge {
                name: name.clone(),
                search_dirs: root_dirs.clone(),
                chain: vec![root_label.clone()],
                depth: DependencyDepth::Direct,
            })
            .collect::<VecDeque<_>>();
        let mut visited = BTreeMap::<PathBuf, ()>::new();
        let mut inspected = 0_usize;
        while let Some(edge) = pending.pop_front() {
            inspected += 1;
            if inspected > self.config.max_dependency_nodes {
                return Err(ToolError::new(format!(
                    "OpenSSL dependency discovery exceeded max_dependency_nodes={}",
                    self.config.max_dependency_nodes
                )));
            }
            let Some(path) = Self::resolve_needed_library(&edge.name, &edge.search_dirs) else {
                notices.push(format!("needed_not_found name={}", edge.name));
                continue;
            };
            let canonical = fs::canonicalize(&path).map_err(|error| {
                ToolError::new(format!("cannot resolve {}: {error}", path.display()))
            })?;
            let chain = Self::chain_with(&edge.chain, &edge.name);
            if Self::is_libssl_name(&edge.name) {
                let confidence = match edge.depth {
                    DependencyDepth::Direct => CONFIDENCE_DIRECT_NEEDED,
                    DependencyDepth::Transitive => CONFIDENCE_TRANSITIVE_NEEDED,
                };
                self.insert_candidate(
                    candidates,
                    &canonical,
                    confidence,
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
            let dependency_dirs = Self::dependency_search_dirs(
                &dependency_dynamic,
                dependency_origin,
                library_search_dirs,
            );
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
        &self,
        image: &ElfImage,
        library_search_dirs: &[PathBuf],
        candidates: &mut BTreeMap<PathBuf, LibraryCandidate>,
        notices: &mut Vec<String>,
    ) -> ToolResult<()> {
        let root_label = Self::file_label(image.path(), "target");
        self.collect_direct_libssl_from_root(
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
        &self,
        image: &ElfImage,
        library_search_dirs: &[PathBuf],
        candidates: &mut BTreeMap<PathBuf, LibraryCandidate>,
        notices: &mut Vec<String>,
    ) -> ToolResult<()> {
        if !Self::is_python_executable(image.path()) {
            return Ok(());
        }
        let Some(extension_path) = Self::python_ssl_extension_path(image.path(), notices)? else {
            return Ok(());
        };
        let extension = ElfImage::parse(&extension_path)?;
        let root_label = format!("{}:_ssl", Self::file_label(image.path(), "python"));
        self.collect_direct_libssl_from_root(
            &extension,
            library_search_dirs,
            candidates,
            notices,
            CONFIDENCE_PYTHON_SSL_NEEDED,
            &root_label,
            Some(format!("python_ssl_extension={}", extension_path.display())),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_direct_libssl_from_root(
        &self,
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
        let search_dirs = Self::dependency_search_dirs(&dynamic, origin, library_search_dirs);
        for needed in &dynamic.needed {
            if !Self::is_libssl_name(needed) {
                continue;
            }
            let Some(path) = Self::resolve_needed_library(needed, &search_dirs) else {
                notices.push(format!("needed_not_found name={needed}"));
                continue;
            };
            let canonical = fs::canonicalize(&path).map_err(|error| {
                ToolError::new(format!("cannot resolve {}: {error}", path.display()))
            })?;
            let mut note = format!("dependency_chain={root_label} -> {needed}");
            if let Some(extra) = extra_note.as_deref() {
                note.push(' ');
                note.push_str(extra);
            }
            self.insert_candidate(candidates, &canonical, confidence, Some(note))?;
        }
        Ok(())
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

    fn dependency_search_dirs(
        dynamic: &DynamicInfo,
        origin: &Path,
        library_search_dirs: &[PathBuf],
    ) -> Vec<PathBuf> {
        let mut dirs = dynamic
            .rpath
            .iter()
            .chain(dynamic.runpath.iter())
            .map(|entry| Self::expand_origin(entry, origin))
            .collect::<Vec<_>>();
        dirs.extend(library_search_dirs.iter().cloned());
        dirs.extend(SYSTEM_LIBRARY_DIRS.iter().map(PathBuf::from));
        dirs
    }

    fn expand_origin(entry: &str, origin: &Path) -> PathBuf {
        entry.strip_prefix(ORIGIN_TOKEN).map_or_else(
            || PathBuf::from(entry),
            |rest| origin.join(rest.trim_start_matches('/')),
        )
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

    fn is_python_executable(path: &Path) -> bool {
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            return false;
        };
        name.strip_prefix("python").is_some_and(|rest| {
            rest.is_empty()
                || rest
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_digit())
        })
    }

    fn rank(confidence: &str) -> u8 {
        match confidence {
            CONFIDENCE_USER_SPECIFIED => 3,
            CONFIDENCE_DIRECT_NEEDED | CONFIDENCE_PYTHON_SSL_NEEDED => 2,
            CONFIDENCE_TRANSITIVE_NEEDED => 1,
            _ => 0,
        }
    }

    fn chain_with(chain: &[String], name: &str) -> Vec<String> {
        let mut next = chain.to_vec();
        next.push(name.to_string());
        next
    }

    fn file_label(path: &Path, fallback: &str) -> String {
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(fallback)
            .to_string()
    }
}
