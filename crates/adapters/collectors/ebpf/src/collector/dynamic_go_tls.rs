//! Cache-backed dynamic Go TLS probe attachment.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use collector_instance::CollectorError;
use config_core::daemon::{PayloadTlsCaptureBackend, PayloadTlsConfig};

use crate::loader::EbpfRuntime;

use super::{EbpfCollector, loader_error};

#[derive(Debug)]
pub(super) struct DynamicGoTlsAttacher {
    enabled: bool,
    cache: BTreeMap<BinaryCacheKey, CachedAttach>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct BinaryCacheKey {
    path: PathBuf,
    len: u64,
    modified: Option<(u64, u32)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CachedAttach {
    Attached,
    Unsupported,
}

impl DynamicGoTlsAttacher {
    pub(super) fn new(config: &PayloadTlsConfig) -> Self {
        Self {
            enabled: config.enabled && config.capture_backend == PayloadTlsCaptureBackend::TlsSync,
            cache: BTreeMap::new(),
        }
    }

    fn attach_if_supported(
        &mut self,
        runtime: &mut EbpfRuntime,
        binary_path: &Path,
    ) -> Result<(), CollectorError> {
        if !self.enabled {
            return Ok(());
        }
        let Some(key) = binary_cache_key(binary_path) else {
            return Ok(());
        };
        if self.cache.contains_key(&key) {
            return Ok(());
        }
        let attached = runtime
            .attach_go_tls_executable(&key.path)
            .map_err(loader_error)?;
        let status = if attached {
            CachedAttach::Attached
        } else {
            CachedAttach::Unsupported
        };
        self.cache.insert(key, status);
        Ok(())
    }
}

impl EbpfCollector {
    pub fn attach_dynamic_go_tls(&mut self, binary_path: &Path) -> Result<(), CollectorError> {
        let Some(runtime) = self.runtime.as_mut() else {
            return Ok(());
        };
        self.dynamic_go_tls
            .attach_if_supported(runtime, binary_path)
    }
}

fn binary_cache_key(path: &Path) -> Option<BinaryCacheKey> {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let metadata = std::fs::metadata(&path).ok()?;
    Some(BinaryCacheKey {
        path,
        len: metadata.len(),
        modified: metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| (duration.as_secs(), duration.subsec_nanos())),
    })
}
