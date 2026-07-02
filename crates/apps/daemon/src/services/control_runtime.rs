//! Runtime registry for synchronous control-decision plugins.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use control_contract::reply::ControlError;
use plugin_system::{
    ControlDecider, ControlDecisionBudget, ControlDecisionRequest, ControlDecisionResponse,
    PluginCommandBudget, PluginCommandRequest, PluginCommandResponse, PluginInstanceStatus,
    PluginLifecycleState, PluginPurpose, PluginRuntimeError,
};

#[derive(Clone)]
pub(in crate::services) struct ControlPluginRuntime {
    deciders: Arc<RwLock<Vec<Arc<ControlDeciderSlot>>>>,
    next_instance_index: Arc<AtomicU64>,
}

impl ControlPluginRuntime {
    pub(in crate::services) fn new() -> Self {
        Self {
            deciders: Arc::new(RwLock::new(Vec::new())),
            next_instance_index: Arc::new(AtomicU64::new(1)),
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
        let instance_index = self.next_instance_index.fetch_add(1, Ordering::Relaxed);
        let status = active_status(decider.as_ref(), 0, 0, None, warnings.clone());
        deciders.push(Arc::new(ControlDeciderSlot::new(
            decider,
            warnings,
            instance_index,
        )));
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
        slot.active.store(false, Ordering::Relaxed);
        let mut status = slot.status();
        status.state = PluginLifecycleState::Stopped;
        Ok(status)
    }

    pub(in crate::services) fn decide(
        &self,
        instance_id_or_index: &str,
        request: ControlDecisionRequest,
        budget: ControlDecisionBudget,
    ) -> Result<ControlDecisionResponse, PluginRuntimeError> {
        let Some(slot) = self.find_slot(instance_id_or_index) else {
            return Err(PluginRuntimeError::new(
                "plugin_runtime",
                format!("plugin instance {instance_id_or_index} not found"),
            ));
        };
        if !slot.active.load(Ordering::Relaxed) {
            return Err(PluginRuntimeError::new(
                "plugin_runtime",
                format!("plugin instance {instance_id_or_index} is not active"),
            ));
        }
        let decision = catch_unwind(AssertUnwindSafe(|| slot.decider.decide(request, budget)))
            .map_err(|_| {
                PluginRuntimeError::new("plugin_panic", "control plugin panicked during decision")
            });
        match decision {
            Ok(Ok(response)) => {
                slot.observed_records.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut last_error) = slot.last_error.lock() {
                    *last_error = None;
                }
                Ok(response)
            }
            Ok(Err(error)) | Err(error) => {
                slot.dropped_records.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut last_error) = slot.last_error.lock() {
                    *last_error = Some(format!("{}: {}", error.code, error.message));
                }
                Err(error)
            }
        }
    }

    pub(in crate::services) fn handle_command(
        &self,
        instance_id: &str,
        request: PluginCommandRequest,
        budget: PluginCommandBudget,
    ) -> Result<PluginCommandResponse, PluginRuntimeError> {
        let Some(slot) = self.find_slot(instance_id) else {
            return Err(PluginRuntimeError::new(
                "plugin_runtime",
                format!("plugin instance {instance_id} not found"),
            ));
        };
        if !slot.active.load(Ordering::Relaxed) {
            return Err(PluginRuntimeError::new(
                "plugin_runtime",
                format!("plugin instance {instance_id} is not active"),
            ));
        }
        match slot.decider.handle_command(request, budget) {
            Ok(response) => {
                if let Ok(mut last_error) = slot.last_error.lock() {
                    *last_error = None;
                }
                Ok(response)
            }
            Err(error) => {
                if let Ok(mut last_error) = slot.last_error.lock() {
                    *last_error = Some(format!("{}: {}", error.code, error.message));
                }
                Err(error)
            }
        }
    }

    pub(in crate::services) fn instance_concurrency_limit(&self, instance_id: &str) -> Option<u32> {
        let Some(slot) = self.find_slot(instance_id) else {
            return None;
        };
        Some(slot.decider.instance_concurrency_limit())
    }

    pub(in crate::services) fn is_instance_index_active(&self, instance_index: u64) -> bool {
        let deciders = self
            .deciders
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        deciders.iter().any(|slot| {
            slot.instance_index == instance_index && slot.active.load(Ordering::Relaxed)
        })
    }

    pub(in crate::services) fn is_instance_active(&self, instance_id_or_index: &str) -> bool {
        self.find_slot(instance_id_or_index)
            .is_some_and(|slot| slot.active.load(Ordering::Relaxed))
    }

    fn find_slot(&self, instance_id_or_index: &str) -> Option<Arc<ControlDeciderSlot>> {
        let deciders = self
            .deciders
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        deciders
            .iter()
            .find(|slot| slot.decider.instance_id() == instance_id_or_index)
            .cloned()
            .or_else(|| {
                instance_id_or_index.parse::<u64>().ok().and_then(|index| {
                    deciders
                        .iter()
                        .find(|slot| slot.instance_index == index)
                        .cloned()
                })
            })
    }
}

struct ControlDeciderSlot {
    decider: Box<dyn ControlDecider>,
    instance_index: u64,
    active: AtomicBool,
    observed_records: AtomicU64,
    dropped_records: AtomicU64,
    last_error: Mutex<Option<String>>,
    warnings: Vec<String>,
}

impl ControlDeciderSlot {
    fn new(decider: Box<dyn ControlDecider>, warnings: Vec<String>, instance_index: u64) -> Self {
        Self {
            decider,
            instance_index,
            active: AtomicBool::new(true),
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
