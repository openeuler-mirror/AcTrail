//! Prepare one fixed runtime and retain its resources or failure until shutdown.

use super::*;

#[derive(Default)]
pub(super) enum RuntimePreparation {
    #[default]
    Unprepared,
    NotRequested,
    Failed(CollectorError),
    Ready(Box<EbpfRuntime>),
}

impl RuntimePreparation {
    pub(super) fn as_ref(&self) -> Option<&EbpfRuntime> {
        match self {
            Self::Ready(runtime) => Some(runtime),
            _ => None,
        }
    }

    pub(super) fn as_mut(&mut self) -> Option<&mut EbpfRuntime> {
        match self {
            Self::Ready(runtime) => Some(runtime),
            _ => None,
        }
    }

    pub(super) fn failure(&self) -> Option<&CollectorError> {
        match self {
            Self::Failed(error) => Some(error),
            _ => None,
        }
    }
}

impl EbpfCollector {
    pub fn has_capability_requests(&self, requests: &[CapabilityRequest]) -> bool {
        requests.iter().any(|request| {
            request.mode != RequestMode::Disabled
                && self
                    .probe_result
                    .descriptor
                    .capabilities
                    .iter()
                    .any(|descriptor| descriptor.capability == request.capability)
        })
    }

    /// Prepare the configured union once. Profile requirements are checked
    /// separately so an unavailable candidate cannot discard a loaded runtime.
    pub fn prepare_capabilities(
        &mut self,
        requests: &[CapabilityRequest],
    ) -> Result<(), CollectorError> {
        match &self.runtime {
            RuntimePreparation::Ready(_) | RuntimePreparation::NotRequested => return Ok(()),
            RuntimePreparation::Failed(error) => return Err(error.clone()),
            RuntimePreparation::Unprepared => (),
        }
        let requests = self.effective_preflight_requests(requests);
        if requests.is_empty() && !self.loader.requires_lsm() {
            self.runtime = RuntimePreparation::NotRequested;
            return Ok(());
        }
        let result = self.load_configured_runtime(&requests);
        match result {
            Ok(runtime) => {
                self.file_tracker = FileTracker::new(
                    self.loader.config().ipc_lineage,
                    runtime.attach_plan.mcp_stdio_enabled(),
                )
                .with_file_capture(runtime.attach_plan.file_capture_enabled())
                .with_file_collection(*self.loader.file_collection());
                self.runtime = RuntimePreparation::Ready(Box::new(runtime));
                Ok(())
            }
            Err(error) => {
                self.runtime = RuntimePreparation::Failed(error.clone());
                Err(error)
            }
        }
    }

    fn load_configured_runtime(
        &self,
        requests: &[CapabilityRequest],
    ) -> Result<EbpfRuntime, CollectorError> {
        if let Some(reason) = &self.probe_result.reason_unavailable {
            return Err(CollectorError::new("ebpf_preflight", reason.clone()));
        }
        let plan = AttachPlan::from_requests(
            requests,
            self.loader.config(),
            self.loader.payload_config(),
            self.semantic_projection_enabled,
            self.loader.file_collection(),
            self.loader.file_directory_observation(),
            self.loader.file_tty_observation(),
        );
        self.loader
            .load_runtime_with_plan(&plan)
            .map_err(loader_error)
    }

    /// Read the preparation result without loading or detaching any program.
    pub fn check_preflight_capabilities(
        &self,
        requests: &[CapabilityRequest],
    ) -> Result<(), CollectorError> {
        if let Some(error) = self.runtime.failure() {
            return Err(error.clone());
        }
        let requests = self.effective_preflight_requests(requests);
        if requests.is_empty() {
            return Err(CollectorError::new(
                "ebpf_preflight",
                "capture profile has no host eBPF capabilities",
            ));
        }
        if let Some(request) = requests.iter().find(|request| {
            request.mode == RequestMode::Required
                && !supported_required_capability(&request.capability, self.loader.payload_config())
        }) {
            return Err(CollectorError::new(
                "ebpf_preflight",
                format!(
                    "configured collector does not support required capability {:?}",
                    request.capability
                ),
            ));
        }
        self.ensure_required_capabilities_attached(&requests)
    }

    fn effective_preflight_requests(
        &self,
        requests: &[CapabilityRequest],
    ) -> Vec<CapabilityRequest> {
        let mut effective = BTreeMap::<Capability, RequestMode>::new();
        for request in requests.iter().filter(|request| {
            request.mode != RequestMode::Disabled
                && self
                    .probe_result
                    .descriptor
                    .capabilities
                    .iter()
                    .any(|descriptor| descriptor.capability == request.capability)
        }) {
            effective
                .entry(request.capability.clone())
                .and_modify(|mode| {
                    if request.mode == RequestMode::Required {
                        *mode = RequestMode::Required;
                    }
                })
                .or_insert(request.mode);
        }
        effective
            .into_iter()
            .map(|(capability, mode)| CapabilityRequest { capability, mode })
            .collect()
    }

    pub(super) fn ensure_runtime_for_requests(
        &mut self,
        requests: &[CapabilityRequest],
    ) -> Result<(), CollectorError> {
        // Standalone collectors may prepare on their first binding. The daemon
        // always seals preparation at startup, including an empty configuration.
        if matches!(self.runtime, RuntimePreparation::Unprepared) {
            self.prepare_capabilities(requests)?;
        }
        if let Some(error) = self.runtime.failure() {
            return Err(error.clone());
        }
        self.ensure_required_capabilities_attached(requests)
    }

    fn ensure_required_capabilities_attached(
        &self,
        requests: &[CapabilityRequest],
    ) -> Result<(), CollectorError> {
        let runtime = self.runtime_ref()?;
        if let Some(missing) = requests.iter().find(|request| {
            request.mode == RequestMode::Required
                && !runtime
                    .attached_capabilities()
                    .contains(&request.capability)
        }) {
            return Err(CollectorError::new(
                "bind_trace",
                format!(
                    "required capability {:?} is unavailable in the runtime prepared at startup; update the daemon configuration and restart",
                    missing.capability
                ),
            ));
        }
        Ok(())
    }
}
