//! Runtime registry for synchronous control-decision plugins.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use control_contract::reply::ControlError;
use plugin_system::{
    ControlDecider, ControlDecisionBudget, ControlDecisionRequest, ControlDecisionResponse,
    PluginInstanceStatus, PluginLifecycleState, PluginPurpose, PluginRuntimeError,
};

#[derive(Clone)]
pub(in crate::services) struct ControlPluginRuntime {
    deciders: Arc<RwLock<Vec<Arc<ControlDeciderSlot>>>>,
}

impl ControlPluginRuntime {
    pub(in crate::services) fn new() -> Self {
        Self {
            deciders: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub(in crate::services) fn plugin_statuses(&self) -> Vec<PluginInstanceStatus> {
        self.deciders
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|slot| slot.status())
            .collect()
    }

    pub(in crate::services) fn add_decider(
        &mut self,
        decider: Box<dyn ControlDecider>,
        warnings: Vec<String>,
    ) -> Result<PluginInstanceStatus, ControlError> {
        let instance_id = decider.instance_id().to_string();
        if instance_id.trim().is_empty() {
            return Err(ControlError::new(
                "plugin_runtime",
                "plugin instance id must not be empty",
            ));
        }
        let mut deciders = self
            .deciders
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if deciders
            .iter()
            .any(|slot| slot.decider.instance_id() == instance_id)
        {
            return Err(ControlError::new(
                "plugin_runtime",
                format!("plugin instance {instance_id} already exists"),
            ));
        }
        let status = active_status(decider.as_ref(), 0, 0, None, warnings.clone());
        deciders.push(Arc::new(ControlDeciderSlot::new(decider, warnings)));
        Ok(status)
    }

    pub(in crate::services) fn remove_decider(
        &mut self,
        instance_id: &str,
    ) -> Result<PluginInstanceStatus, ControlError> {
        let mut deciders = self
            .deciders
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(index) = deciders
            .iter()
            .position(|slot| slot.decider.instance_id() == instance_id)
        else {
            return Err(ControlError::new(
                "plugin_runtime",
                format!("plugin instance {instance_id} not found"),
            ));
        };
        let slot = deciders.remove(index);
        let mut status = slot.status();
        status.state = PluginLifecycleState::Stopped;
        Ok(status)
    }

    pub(in crate::services) fn decide(
        &self,
        instance_id: &str,
        request: ControlDecisionRequest,
        budget: ControlDecisionBudget,
    ) -> Result<ControlDecisionResponse, PluginRuntimeError> {
        let slot = {
            let deciders = self
                .deciders
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(slot) = deciders
                .iter()
                .find(|slot| slot.decider.instance_id() == instance_id)
                .cloned()
            else {
                return Err(PluginRuntimeError::new(
                    "plugin_runtime",
                    format!("plugin instance {instance_id} not found"),
                ));
            };
            slot
        };
        match slot.decider.decide(request, budget) {
            Ok(response) => {
                slot.observed_records.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut last_error) = slot.last_error.lock() {
                    *last_error = None;
                }
                Ok(response)
            }
            Err(error) => {
                slot.dropped_records.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut last_error) = slot.last_error.lock() {
                    *last_error = Some(format!("{}: {}", error.code, error.message));
                }
                Err(error)
            }
        }
    }

    pub(in crate::services) fn instance_concurrency_limit(&self, instance_id: &str) -> Option<u32> {
        let deciders = self
            .deciders
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        deciders
            .iter()
            .find(|slot| slot.decider.instance_id() == instance_id)
            .map(|slot| slot.decider.instance_concurrency_limit())
    }
}

struct ControlDeciderSlot {
    decider: Box<dyn ControlDecider>,
    observed_records: AtomicU64,
    dropped_records: AtomicU64,
    last_error: Mutex<Option<String>>,
    warnings: Vec<String>,
}

impl ControlDeciderSlot {
    fn new(decider: Box<dyn ControlDecider>, warnings: Vec<String>) -> Self {
        Self {
            decider,
            observed_records: AtomicU64::new(0),
            dropped_records: AtomicU64::new(0),
            last_error: Mutex::new(None),
            warnings,
        }
    }

    fn status(&self) -> PluginInstanceStatus {
        active_status(
            self.decider.as_ref(),
            self.observed_records.load(Ordering::Relaxed),
            self.dropped_records.load(Ordering::Relaxed),
            self.last_error.lock().ok().and_then(|error| error.clone()),
            self.warnings.clone(),
        )
    }
}

fn active_status(
    decider: &dyn ControlDecider,
    observed_records: u64,
    dropped_records: u64,
    last_error: Option<String>,
    warnings: Vec<String>,
) -> PluginInstanceStatus {
    PluginInstanceStatus {
        instance_id: decider.instance_id().to_string(),
        plugin_id: decider.plugin_id().to_string(),
        purpose: PluginPurpose::ControlDecider,
        runtime: decider.runtime_kind(),
        state: PluginLifecycleState::Active,
        host_grants: decider.host_grants(),
        queue_depth: None,
        queue_capacity: None,
        observed_records,
        dropped_records,
        hostcall_metrics: decider
            .hostcall_metrics_source()
            .as_ref()
            .map(|metrics| metrics.snapshot())
            .unwrap_or_default(),
        last_error,
        warnings,
    }
}
