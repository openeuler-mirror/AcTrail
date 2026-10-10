use super::controller::{LsmController, PolicyStores};
use config_core::daemon::{EnforcementDecision, LsmDenySignal};
use plugin_system::{FilePolicyOperation, NetworkPolicyRemoteSelector};
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};

/// Explicit byte encoding keeps the userspace/kernel ABI independent of Rust padding.
#[derive(Clone)]
pub(super) struct PolicySnapshot {
    pub(super) entries: BTreeMap<[u8; 260], [u8; 288]>,
}

impl PolicySnapshot {
    pub(super) fn compile(
        controller: &LsmController,
        stores: &PolicyStores,
    ) -> Result<Self, String> {
        let mut snapshot = stores.file.clone();
        let command = &controller.command_config;
        let network = &controller.network_config;
        for (domain, enabled, default, audit, revision) in [
            (
                4,
                command.enabled,
                command.default_decision,
                command.audit_enabled,
                stores.command.revision(),
            ),
            (
                5,
                network.enabled,
                network.default_decision,
                network.audit_enabled,
                stores.network.revision(),
            ),
        ] {
            let decision = if enabled {
                default
            } else {
                EnforcementDecision::Allow
            };
            let mut value = Self::value(
                controller,
                u32::MAX,
                decision.as_str(),
                decision,
                enabled
                    && audit
                    && (decision == EnforcementDecision::Deny
                        || controller.config.audit_default_allow),
                revision,
                "",
                "",
            )?;
            if !enabled {
                value[8..12].copy_from_slice(&0_u32.to_ne_bytes());
            }
            snapshot.insert(domain, &[], value)?;
        }
        if command.enabled {
            for (rank, rule) in stores
                .command
                .effective_rule_views()
                .into_iter()
                .enumerate()
            {
                if rule
                    .args
                    .as_ref()
                    .is_some_and(|args| args.as_slice() != ["*"])
                {
                    return Err(format!(
                        "LSM command rule {} requires executable-only matching; argument patterns need a separate decision backend",
                        rule.rule_id
                    ));
                }
                let value = Self::value(
                    controller,
                    rank as u32,
                    rule.decision.as_str(),
                    command.default_decision,
                    command.audit_enabled,
                    rule.rule_revision,
                    &rule.rule_id,
                    &rule.owner_instance_id,
                )?;
                let mut path = rule.executable.into_bytes();
                path.push(0);
                snapshot.check_path(controller, &path)?;
                snapshot.insert(4, &path, value)?;
            }
        }
        if network.enabled {
            for (rank, rule) in stores
                .network
                .effective_rule_views()
                .into_iter()
                .enumerate()
            {
                let selector = NetworkPolicyRemoteSelector::parse(&rule.remote)?;
                let (ip, port) = match selector {
                    NetworkPolicyRemoteSelector::Endpoint(SocketAddr::V6(endpoint))
                        if endpoint.scope_id() != 0 =>
                    {
                        return Err(format!(
                            "LSM network rule {} cannot preserve IPv6 scope ID in {}; scoped endpoints cannot be published",
                            rule.rule_id, rule.remote
                        ));
                    }
                    NetworkPolicyRemoteSelector::Endpoint(endpoint) => {
                        (endpoint.ip(), Some(endpoint.port()))
                    }
                    NetworkPolicyRemoteSelector::AnyPort(ip) => (ip, None),
                };
                let mut resource = match ip {
                    IpAddr::V4(ip) => {
                        let mut key = vec![4];
                        key.extend_from_slice(&ip.octets());
                        key
                    }
                    IpAddr::V6(ip) => {
                        let mut key = vec![6];
                        key.extend_from_slice(&ip.octets());
                        key
                    }
                };
                if let Some(port) = port {
                    resource.extend_from_slice(&port.to_be_bytes());
                }
                let value = Self::value(
                    controller,
                    rank as u32,
                    rule.decision.as_str(),
                    network.default_decision,
                    network.audit_enabled,
                    rule.rule_revision,
                    &rule.rule_id,
                    &rule.owner_instance_id,
                )?;
                snapshot.insert(5, &resource, value)?;
            }
        }
        if snapshot.entries.len() > controller.config.policy_max_entries as usize {
            return Err(format!(
                "LSM snapshot requires {} entries, policy_max_entries={}",
                snapshot.entries.len(),
                controller.config.policy_max_entries
            ));
        }
        Ok(snapshot)
    }

    pub(super) fn compile_file(
        controller: &LsmController,
        rules: &crate::policy::file::EnforcementRules,
    ) -> Result<Self, String> {
        let mut snapshot = Self {
            entries: BTreeMap::new(),
        };
        let file = &controller.file_config;
        for (domain, enabled, default, audit, revision) in [
            (
                1,
                controller.file_domain_enabled(1),
                file.default_decision,
                file.audit_enabled,
                rules.revision(),
            ),
            (
                2,
                controller.file_domain_enabled(2),
                file.default_decision,
                file.audit_enabled,
                rules.revision(),
            ),
            (
                3,
                controller.file_domain_enabled(3),
                file.default_decision,
                file.audit_enabled,
                rules.revision(),
            ),
        ] {
            let decision = if enabled {
                default
            } else {
                EnforcementDecision::Allow
            };
            let mut value = Self::value(
                controller,
                u32::MAX,
                decision.as_str(),
                decision,
                enabled
                    && audit
                    && (decision == EnforcementDecision::Deny
                        || controller.config.audit_default_allow),
                revision,
                "",
                "",
            )?;
            if !enabled {
                value[8..12].copy_from_slice(&0_u32.to_ne_bytes());
            }
            snapshot.insert(domain, &[], value)?;
        }
        if file.enabled {
            for (rank, rule) in rules.effective_rule_views().into_iter().enumerate() {
                let domains: &[u32] = match rule.operation {
                    FilePolicyOperation::Any => &[1, 2, 3],
                    FilePolicyOperation::Open => &[1],
                    FilePolicyOperation::Mkdir => &[2],
                    FilePolicyOperation::Rmdir => &[3],
                };
                let domains = domains
                    .iter()
                    .copied()
                    .filter(|domain| controller.file_domain_enabled(*domain))
                    .collect::<Vec<_>>();
                if domains.is_empty() {
                    continue;
                }
                let value = Self::value(
                    controller,
                    rank as u32,
                    rule.decision.as_str(),
                    file.default_decision,
                    file.audit_enabled,
                    rules.revision(),
                    &rule.rule_id,
                    &rule.owner_instance_id,
                )?;
                let recursive = rule.path.strip_suffix("/**");
                let base = recursive.unwrap_or(&rule.path);
                let base = if base.is_empty() { "/" } else { base };
                let mut exact = base.as_bytes().to_vec();
                exact.push(0);
                snapshot.check_path(controller, &exact)?;
                for domain in domains {
                    snapshot.insert(domain, &exact, value)?;
                    if recursive.is_some() {
                        let prefix = format!("{}/", base.trim_end_matches('/'));
                        snapshot.check_path(controller, prefix.as_bytes())?;
                        snapshot.insert(domain, prefix.as_bytes(), value)?;
                        let default = snapshot
                            .entries
                            .get_mut(&Self::key(domain, &[])?)
                            .expect("domain default");
                        let flags = u32::from_ne_bytes(default[8..12].try_into().unwrap()) | 2;
                        default[8..12].copy_from_slice(&flags.to_ne_bytes());
                    }
                }
            }
        }
        Ok(snapshot)
    }

    fn check_path(&self, controller: &LsmController, path: &[u8]) -> Result<(), String> {
        if path.len() > controller.config.path_max_bytes as usize {
            return Err("LSM rule path exceeds access_control.backends.lsm.path_max_bytes".into());
        }
        Ok(())
    }

    fn key(domain: u32, resource: &[u8]) -> Result<[u8; 260], String> {
        if resource.len() > 252 {
            return Err("LSM resource exceeds the kernel LPM limit of 252 bytes".into());
        }
        let mut key = [0; 260];
        key[..4].copy_from_slice(&(32 + resource.len() as u32 * 8).to_ne_bytes());
        key[4..8].copy_from_slice(&domain.to_ne_bytes());
        key[8..8 + resource.len()].copy_from_slice(resource);
        Ok(key)
    }

    fn insert(&mut self, domain: u32, resource: &[u8], value: [u8; 288]) -> Result<(), String> {
        // Effective views are ordered by policy precedence; the first identical key wins.
        self.entries
            .entry(Self::key(domain, resource)?)
            .or_insert(value);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn value(
        controller: &LsmController,
        rank: u32,
        decision: &str,
        default: EnforcementDecision,
        audit: bool,
        revision: u64,
        rule_id: &str,
        owner: &str,
    ) -> Result<[u8; 288], String> {
        let deny = match decision {
            "allow" => false,
            "deny" => true,
            "default" => default == EnforcementDecision::Deny,
            _ => {
                return Err(format!(
                    "LSM rule {rule_id} requires a synchronous userspace decision; use allow or deny"
                ));
            }
        };
        if rule_id.len() >= 128 || owner.len() >= 128 {
            return Err("LSM rule and owner identifiers must be shorter than 128 bytes".into());
        }
        if rule_id.contains('\0') || owner.contains('\0') {
            return Err("LSM rule and owner identifiers must not contain NUL".into());
        }
        let mut value = [0; 288];
        let flags = 8
            | u32::from(audit)
            | if controller.config.deny_on_error {
                4
            } else {
                0
            };
        let signal: u32 = match controller.config.deny_signal {
            LsmDenySignal::None => 0,
            LsmDenySignal::Stop => 19,
            LsmDenySignal::Kill => 9,
        };
        for (offset, field) in [
            (0, rank),
            (4, u32::from(deny)),
            (8, flags),
            (12, signal),
            (16, controller.config.path_max_bytes),
        ] {
            value[offset..offset + 4].copy_from_slice(&field.to_ne_bytes());
        }
        value[24..32].copy_from_slice(&revision.to_ne_bytes());
        value[32..32 + rule_id.len()].copy_from_slice(rule_id.as_bytes());
        value[160..160 + owner.len()].copy_from_slice(owner.as_bytes());
        Ok(value)
    }
}
