//! Loads the configured integrations and assembles the backend of each Trace from them.

use abi_stable::sabi_trait::TD_Opaque;
use checkpoint::{BackendKind, CheckpointBackend, RestorableCheckpointBackend_TO};

use crate::backends::composite::{Child, CompositeBackend};
use crate::backends::dynamic::DynamicIntegration;
use crate::config::BackendEntry;
use crate::coordinator::CoordinatorError;
use crate::scope::BackendGroup;

pub struct CheckpointFactory {
    /// In configuration order, which is the order a Trace's backend calls them.
    backends: Vec<Backend>,
}

struct Backend {
    name: String,
    integration: DynamicIntegration,
}

impl CheckpointFactory {
    pub fn load(entries: &[BackendEntry]) -> Result<Self, CoordinatorError> {
        let mut backends: Vec<Backend> = Vec::with_capacity(entries.len());
        for entry in entries {
            let failed = |message: String| {
                CoordinatorError::new("checkpoint_backend", format!("{}: {message}", entry.name))
            };
            if backends.iter().any(|backend| backend.name == entry.name) {
                return Err(failed("configured twice".to_string()));
            }
            backends.push(Backend {
                name: entry.name.clone(),
                integration: DynamicIntegration::load(&entry.library, &entry.config)
                    .map_err(failed)?,
            });
        }
        Ok(Self { backends })
    }

    /// Whether launch scopes can name this backend.
    pub(crate) fn takes_scopes(&self, name: &str) -> bool {
        self.backends.iter().any(|backend| {
            backend.name == name && backend.integration.kind() == BackendKind::Filesystem
        })
    }

    /// The backend of one Trace: a new instance of each configured backend that takes part. A
    /// filesystem backend takes part when a scope names it; a context backend always does.
    pub fn create(&self, groups: &[BackendGroup]) -> Result<CheckpointBackend, CoordinatorError> {
        let children = self
            .backends
            .iter()
            .filter_map(|backend| {
                let group = groups
                    .iter()
                    .find(|group| group.backend_instance == backend.name);
                let takes_part =
                    group.is_some() || backend.integration.kind() == BackendKind::Context;
                takes_part.then_some((backend, group))
            })
            .map(|(backend, group)| {
                let instance = backend.integration.create().map_err(|message| {
                    CoordinatorError::new(
                        "connect_failed",
                        format!("backend instance {}: {message}", backend.name),
                    )
                })?;
                Ok(Child::new(&backend.name, group, instance))
            })
            .collect::<Result<_, CoordinatorError>>()?;
        Ok(RestorableCheckpointBackend_TO::from_value(
            CompositeBackend::new(children),
            TD_Opaque,
        ))
    }
}
