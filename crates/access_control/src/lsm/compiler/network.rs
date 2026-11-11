//! Compile network scopes into port-prefix buckets with IP longest-prefix lookup.
use super::PolicySnapshot;
use crate::lsm::controller::{LsmController, PolicyStores};
use plugin_system::{NetworkPolicyEndpointSelector, NetworkPolicyOperation};
use std::net::IpAddr;

impl PolicySnapshot {
    pub(super) fn compile_network(
        &mut self,
        controller: &LsmController,
        stores: &PolicyStores,
    ) -> Result<(), String> {
        let config = &controller.network_config;
        for (rank, rule) in stores
            .network
            .effective_rule_views()
            .into_iter()
            .enumerate()
        {
            let scope = NetworkPolicyEndpointSelector::parse(&rule.endpoint)?;
            if scope.scope_id() != 0 {
                return Err(format!(
                    "LSM network rule {} cannot preserve IPv6 scope ID in {}",
                    rule.rule_id, rule.endpoint
                ));
            }
            let domain = match rule.operation {
                NetworkPolicyOperation::Connect => 5,
                NetworkPolicyOperation::Send => 12,
                NetworkPolicyOperation::Bind => 13,
                NetworkPolicyOperation::Listen => 14,
            };
            let value = Self::value(
                controller,
                rank as u32,
                rule.decision.as_str(),
                config.default_decision,
                config.audit_enabled,
                rule.rule_revision,
                &rule.rule_id,
                &rule.owner_instance_id,
            )?;
            let mut bitmap = 0_u32;
            for (port_prefix, port) in scope.port_prefixes() {
                bitmap |= 1 << port_prefix;
                let mut resource = vec![rule.protocol.code(), port_prefix];
                resource.extend_from_slice(&port.to_be_bytes());
                match scope.network() {
                    IpAddr::V4(ip) => {
                        resource.push(4);
                        resource.extend_from_slice(&ip.octets());
                    }
                    IpAddr::V6(ip) => {
                        resource.push(6);
                        resource.extend_from_slice(&ip.octets());
                    }
                }
                let mut key = Self::key(domain, &resource)?;
                key[..4].copy_from_slice(&(32_u32 + 40 + u32::from(scope.prefix())).to_ne_bytes());
                self.entries.entry(key).or_insert(value);
            }
            let fallback = self
                .entries
                .get_mut(&Self::key(domain, &[])?)
                .expect("network default");
            bitmap |= u32::from_ne_bytes(fallback[20..24].try_into().unwrap());
            fallback[20..24].copy_from_slice(&bitmap.to_ne_bytes());
            if self.entries.len() > controller.config.policy_max_entries as usize {
                return Err(format!(
                    "LSM snapshot requires more than policy_max_entries={}",
                    controller.config.policy_max_entries
                ));
            }
        }
        Ok(())
    }
}
