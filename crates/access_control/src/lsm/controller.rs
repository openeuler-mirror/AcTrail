use super::compiler::PolicySnapshot;
use super::publisher::PolicyPublisher;
use crate::policy::{
    command::CommandPolicyStore, file::EnforcementRules, network::NetworkPolicyStore,
};
use config_core::daemon::{
    CommandControlConfig, EnforcementConfig, FileControlConfig, LsmConfig, NetworkControlConfig,
};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

#[derive(Clone)]
pub(super) struct PolicyStores {
    pub(super) file: PolicySnapshot,
    pub(super) command: CommandPolicyStore,
    pub(super) network: NetworkPolicyStore,
}

pub(super) struct ControllerState {
    pub(super) stores: PolicyStores,
    publisher: Weak<PolicyPublisher>,
}

/// Owns LSM command/network policies and derived file entries.
/// The file service owns file rules; its successful publications replace the cached entries.
/// Reusing those entries avoids recompiling file rules during command/network updates.
pub struct LsmController {
    pub(super) config: LsmConfig,
    pub(super) file_config: EnforcementConfig,
    file_operations: FileControlConfig,
    pub(super) command_config: CommandControlConfig,
    pub(super) network_config: NetworkControlConfig,
    state: Mutex<ControllerState>,
}

impl LsmController {
    pub(super) fn host_error(message: impl Into<String>) -> plugin_system::PluginRuntimeError {
        plugin_system::PluginRuntimeError::new("lsm_policy", message)
    }
    pub fn new(
        config: LsmConfig,
        file_config: EnforcementConfig,
        file_operations: FileControlConfig,
        command_config: CommandControlConfig,
        network_config: NetworkControlConfig,
    ) -> Result<Arc<Self>, String> {
        let command = if command_config.enabled {
            CommandPolicyStore::load(&command_config.rules_path)?
        } else {
            CommandPolicyStore::empty()
        };
        let network = if network_config.enabled {
            NetworkPolicyStore::load(&network_config.rules_path)?
        } else {
            NetworkPolicyStore::empty()
        };
        let controller = Arc::new(Self {
            config,
            file_config,
            file_operations,
            command_config,
            network_config,
            state: Mutex::new(ControllerState {
                stores: PolicyStores {
                    file: PolicySnapshot {
                        entries: Default::default(),
                    },
                    command,
                    network,
                },
                publisher: Weak::new(),
            }),
        });
        controller.publish_file_rules(&EnforcementRules::empty_for_lsm())?;
        Ok(controller)
    }

    pub fn command_enabled(&self) -> bool {
        self.command_config.enabled
    }
    pub fn network_enabled(&self) -> bool {
        self.network_config.enabled
    }

    pub(super) fn file_domain_enabled(&self, domain: u32) -> bool {
        self.file_config.enabled
            && match domain {
                1 => self.file_operations.open.is_lsm(),
                2 => self.file_operations.mkdir.is_lsm(),
                3 => self.file_operations.rmdir.is_lsm(),
                _ => false,
            }
    }

    pub fn validate_file_rules(&self, rules: &EnforcementRules) -> Result<(), String> {
        let mut state = self.lock()?;
        let mut stores = state.stores.clone();
        stores.file = PolicySnapshot::compile_file(self, rules)?;
        self.commit(&mut state, stores, false)
    }

    pub fn publish_file_rules(&self, rules: &EnforcementRules) -> Result<(), String> {
        let mut state = self.lock()?;
        let mut stores = state.stores.clone();
        stores.file = PolicySnapshot::compile_file(self, rules)?;
        self.commit(&mut state, stores, true)
    }

    pub(super) fn lock(&self) -> Result<MutexGuard<'_, ControllerState>, String> {
        self.state
            .lock()
            .map_err(|_| "LSM policy controller lock poisoned; retaining published policy".into())
    }

    pub(super) fn bind(&self, publisher: &Arc<PolicyPublisher>) -> Result<(), String> {
        let mut state = self.lock()?;
        if state.publisher.upgrade().is_some() {
            return Err("LSM controller already attached to an observation runtime".into());
        }
        publisher.publish(&PolicySnapshot::compile(self, &state.stores)?)?;
        state.publisher = Arc::downgrade(publisher);
        Ok(())
    }

    pub(super) fn commit(
        &self,
        state: &mut ControllerState,
        stores: PolicyStores,
        publish: bool,
    ) -> Result<(), String> {
        let snapshot = PolicySnapshot::compile(self, &stores)?;
        if publish {
            if let Some(publisher) = state.publisher.upgrade() {
                publisher.publish(&snapshot)?;
            }
            state.stores = stores;
        }
        Ok(())
    }

    pub fn remove_owner(&self, owner: &str) -> Result<(), String> {
        let mut state = self.lock()?;
        let mut stores = state.stores.clone();
        stores.command.remove_owner(owner)?;
        stores.network.remove_owner(owner)?;
        self.commit(&mut state, stores, true)
    }
}
