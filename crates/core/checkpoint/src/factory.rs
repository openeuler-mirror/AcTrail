//! Loads the configured integrations and assembles the backend of each Trace from them.

use checkpoint::{BackendInstanceConfig, BackendKind};

use crate::backends::composite::{Child, CompositeBackend};
use crate::backends::dynamic::DynamicIntegration;
use crate::config::BackendEntry;
use crate::coordinator::CoordinatorError;
use crate::scope::BackendGroup;

pub struct CheckpointFactory {
    /// Loaded implementations available to each launch.
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

    /// Creates exactly the selected instances, in launch configuration order. Private
    /// Trace configuration files are opened only by the selected integration's creation entry.
    pub(crate) fn create(
        &self,
        selections: &[BackendInstanceConfig],
        groups: &[BackendGroup],
    ) -> Result<CompositeBackend, CoordinatorError> {
        let invalid = |message| CoordinatorError::new("checkpoint_backend", message);
        if selections.is_empty() {
            return Err(invalid(
                "the checkpoint configuration has no backends".to_string(),
            ));
        }
        // Resolve the complete selection before creating any backend resources.
        let mut selected = Vec::with_capacity(selections.len());
        for (index, selection) in selections.iter().enumerate() {
            if selections[..index]
                .iter()
                .any(|other| other.name == selection.name)
            {
                return Err(invalid(format!(
                    "backend {} is selected twice",
                    selection.name
                )));
            }
            let backend = self
                .backends
                .iter()
                .find(|backend| backend.name == selection.name)
                .ok_or_else(|| invalid(format!("backend {} is not loaded", selection.name)))?;
            let group = groups
                .iter()
                .find(|group| group.backend_instance == selection.name);
            if (backend.integration.kind() == BackendKind::Filesystem) != group.is_some() {
                return Err(invalid(format!(
                    "backend {} requires scopes exactly when it is a filesystem backend",
                    selection.name
                )));
            }
            selected.push((backend, group, selection));
        }
        for group in groups {
            if !selections
                .iter()
                .any(|selection| selection.name == group.backend_instance)
            {
                return Err(invalid(format!(
                    "scope backend {} is not selected",
                    group.backend_instance
                )));
            }
        }
        let children = selected
            .iter()
            .map(|(backend, group, selection)| {
                let instance = backend
                    .integration
                    .create(selection.config.as_deref())
                    .map_err(|message| {
                        CoordinatorError::new(
                            "connect_failed",
                            format!("backend instance {}: {message}", backend.name),
                        )
                    })?;
                Ok(Child::new(
                    &backend.name,
                    backend.integration.kind(),
                    *group,
                    instance,
                ))
            })
            .collect::<Result<_, CoordinatorError>>()?;
        Ok(CompositeBackend::new(children))
    }
}
