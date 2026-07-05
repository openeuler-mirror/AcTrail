//! Shared-library probe metadata for TLS stacks with stable exported IO symbols.

use std::path::{Path, PathBuf};

pub(crate) const GNUTLS_NAME: &str = "gnutls";
pub(crate) const GNUTLS_RESOLVER: &str = "gnutls-symbols";
pub(crate) const GNUTLS_RECORD_SEND: &str = "gnutls_record_send";
pub(crate) const GNUTLS_RECORD_RECV: &str = "gnutls_record_recv";
pub(crate) const GNUTLS_SYMBOLS: &[&str] = &[GNUTLS_RECORD_SEND, GNUTLS_RECORD_RECV];

pub(crate) const NSS_NAME: &str = "nss";
pub(crate) const NSS_RESOLVER: &str = "nss-nspr-symbols";
pub(crate) const NSPR_PR_WRITE: &str = "PR_Write";
pub(crate) const NSPR_PR_SEND: &str = "PR_Send";
pub(crate) const NSPR_PR_READ: &str = "PR_Read";
pub(crate) const NSPR_PR_RECV: &str = "PR_Recv";
pub(crate) const NSS_SYMBOLS: &[&str] = &[NSPR_PR_WRITE, NSPR_PR_SEND, NSPR_PR_READ, NSPR_PR_RECV];

pub(crate) fn explicit_or_current_shared_library_candidates(
    target_path: &Path,
    libraries: &[PathBuf],
    soname_prefix: &str,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for path in libraries {
        push_candidate(&mut candidates, path.clone());
    }
    if is_matching_shared_object(target_path, soname_prefix) {
        push_candidate(&mut candidates, target_path.to_path_buf());
    }
    candidates
}

fn push_candidate(candidates: &mut Vec<PathBuf>, path: PathBuf) {
    if !candidates.iter().any(|existing| existing == &path) {
        candidates.push(path);
    }
}

fn is_matching_shared_object(path: &Path, soname_prefix: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(soname_prefix) && name.contains(".so"))
}
