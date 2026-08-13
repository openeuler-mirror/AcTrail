//! Bounded startup teardown for independent static eBPF links.

use std::thread;

use config_core::daemon::MAX_EBPF_PREFLIGHT_LINK_TEARDOWN_WORKERS;
use libbpf_rs::Link;

use super::LoaderError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct StaticLinkTeardown {
    worker_count: usize,
}

impl StaticLinkTeardown {
    pub(super) fn new(worker_count: u32) -> Result<Self, LoaderError> {
        if worker_count == 0 || worker_count > MAX_EBPF_PREFLIGHT_LINK_TEARDOWN_WORKERS {
            return Err(LoaderError::new(
                "preflight_link_teardown_workers",
                format!(
                    "worker count must be between 1 and {MAX_EBPF_PREFLIGHT_LINK_TEARDOWN_WORKERS}, got {worker_count}"
                ),
            ));
        }
        Ok(Self {
            worker_count: worker_count as usize,
        })
    }

    pub(super) fn drop_all(&self, links: Vec<Link>) -> Result<(), LoaderError> {
        let worker_count = self.worker_count.min(links.len());
        if worker_count <= 1 {
            drop(links);
            return Ok(());
        }

        let bucket_capacity = links.len().div_ceil(worker_count);
        let mut buckets = (0..worker_count)
            .map(|_| Vec::with_capacity(bucket_capacity))
            .collect::<Vec<_>>();
        for (index, link) in links.into_iter().enumerate() {
            buckets[index % worker_count].push(link);
        }

        thread::scope(|scope| {
            let mut handles = Vec::with_capacity(worker_count);
            let mut spawn_error = None;
            for (index, bucket) in buckets.into_iter().enumerate() {
                if spawn_error.is_some() {
                    drop(bucket);
                    continue;
                }
                match thread::Builder::new()
                    .name(format!("actrail-ebpf-link-drop-{index}"))
                    .spawn_scoped(scope, move || drop(bucket))
                {
                    Ok(handle) => handles.push(handle),
                    Err(error) => spawn_error = Some(error),
                }
            }

            let mut worker_panicked = false;
            for handle in handles {
                if handle.join().is_err() {
                    worker_panicked = true;
                }
            }
            if let Some(error) = spawn_error {
                return Err(LoaderError::new(
                    "preflight_link_teardown_spawn",
                    error.to_string(),
                ));
            }
            if worker_panicked {
                return Err(LoaderError::new(
                    "preflight_link_teardown_join",
                    "static eBPF link teardown worker panicked",
                ));
            }
            Ok(())
        })
    }
}
