//! Cache-backed attachment for detector-produced TLS probe plans.

use std::collections::BTreeSet;
use std::os::unix::fs::MetadataExt;

use collector_instance::CollectorError;
use model_core::binary_identity::BinaryIdentity;

use crate::loader::{DynamicTlsProbePlan, EbpfRuntime};

use super::{EbpfCollector, loader_error};

#[derive(Debug, Default)]
pub(super) struct DynamicTlsAttacher {
    attached: BTreeSet<(
        u64,
        u64,
        u64,
        i64,
        i64,
        i64,
        i64,
        BinaryIdentity,
        String,
        String,
    )>,
}

impl DynamicTlsAttacher {
    fn attach(
        &mut self,
        runtime: &mut EbpfRuntime,
        plan: &DynamicTlsProbePlan,
    ) -> Result<(), CollectorError> {
        let metadata = std::fs::metadata(&plan.binary).map_err(|error| {
            CollectorError::new(
                "attach_dynamic_tls",
                format!(
                    "read probe binary metadata {}: {error}",
                    plan.binary.display()
                ),
            )
        })?;
        let key = (
            metadata.dev(),
            metadata.ino(),
            metadata.size(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec(),
            plan.binary_identity.clone(),
            plan.provider.clone(),
            plan.points.clone(),
        );
        if self.attached.contains(&key) {
            return Ok(());
        }
        runtime
            .attach_dynamic_tls_plan(plan)
            .map_err(loader_error)?;
        self.attached.insert(key);
        Ok(())
    }
}

impl EbpfCollector {
    pub fn attach_dynamic_tls_plan(
        &mut self,
        plan: &DynamicTlsProbePlan,
    ) -> Result<(), CollectorError> {
        let Some(runtime) = self.runtime.as_mut() else {
            return Err(CollectorError::new(
                "attach_dynamic_tls",
                "eBPF runtime is not loaded",
            ));
        };
        self.dynamic_tls.attach(runtime, plan)
    }
}
