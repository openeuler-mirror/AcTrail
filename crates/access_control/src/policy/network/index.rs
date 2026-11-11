//! Derived IP-prefix and disjoint-port indexes rebuilt when rules are published.
use super::rule::StoredNetworkRule;
use plugin_system::{
    NetworkPolicyEndpointSelector as Selector, NetworkPolicyOperation as Operation,
    NetworkPolicyProtocol as Protocol,
};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, SocketAddr};

type AddressKey = (Operation, u8, IpAddr, u32);

#[derive(Clone, Debug, Default)]
pub(super) struct NetworkRuleIndex {
    pub(super) rules: Vec<StoredNetworkRule>,
    prefixes: BTreeMap<(Operation, bool), BTreeSet<Reverse<u8>>>,
    addresses: BTreeMap<AddressKey, BTreeMap<Protocol, PortIndex>>,
}

impl NetworkRuleIndex {
    pub(super) fn build(mut rules: Vec<StoredNetworkRule>) -> Self {
        // The same order supplies userspace winners and kernel snapshot ranks.
        rules.sort_by_key(|rule| {
            (
                Reverse(rule.endpoint.prefix()),
                rule.endpoint.ports().1 - rule.endpoint.ports().0,
                rule.protocol == Protocol::Any,
            )
        });
        let mut index = Self {
            rules,
            ..Self::default()
        };
        let mut groups: BTreeMap<(AddressKey, Protocol), Vec<usize>> = BTreeMap::new();
        for (rank, rule) in index.rules.iter().enumerate() {
            let scope = rule.endpoint;
            index
                .prefixes
                .entry((rule.operation, scope.network().is_ipv4()))
                .or_default()
                .insert(Reverse(scope.prefix()));
            groups
                .entry((
                    (
                        rule.operation,
                        scope.prefix(),
                        scope.network(),
                        scope.scope_id(),
                    ),
                    rule.protocol,
                ))
                .or_default()
                .push(rank);
        }
        for ((key, protocol), ranks) in groups {
            index
                .addresses
                .entry(key)
                .or_default()
                .insert(protocol, PortIndex::build(&index.rules, ranks));
        }
        index
    }

    pub(super) fn find(
        &self,
        operation: Operation,
        protocol: Protocol,
        endpoint: SocketAddr,
    ) -> Option<&StoredNetworkRule> {
        let scope = Selector::from_endpoint(endpoint);
        for Reverse(prefix) in self.prefixes.get(&(operation, endpoint.is_ipv4()))? {
            let network = Selector::network_at(endpoint.ip(), *prefix);
            let scopes =
                std::iter::once(0).chain((scope.scope_id() != 0).then_some(scope.scope_id()));
            let mut selected = None;
            for scope_id in scopes {
                if let Some(protocols) =
                    self.addresses.get(&(operation, *prefix, network, scope_id))
                {
                    let explicit = protocols
                        .get(&protocol)
                        .and_then(|ports| ports.find(endpoint.port()));
                    let any = protocols
                        .get(&Protocol::Any)
                        .and_then(|ports| ports.find(endpoint.port()));
                    selected = selected.into_iter().chain(explicit).chain(any).min();
                }
            }
            if let Some(rank) = selected {
                return self.rules.get(rank);
            }
        }
        None
    }
}

#[derive(Clone, Debug)]
struct PortIndex(BTreeMap<u16, Option<usize>>);

impl PortIndex {
    fn build(rules: &[StoredNetworkRule], ranks: Vec<usize>) -> Self {
        let mut events: BTreeMap<u32, Vec<(usize, bool)>> = BTreeMap::new();
        for rank in ranks {
            let (first, last) = rules[rank].endpoint.ports();
            events
                .entry(u32::from(first))
                .or_default()
                .push((rank, true));
            events
                .entry(u32::from(last) + 1)
                .or_default()
                .push((rank, false));
        }
        let mut active = BTreeSet::new();
        let mut segments = BTreeMap::new();
        for (port, changes) in events {
            if port > u32::from(u16::MAX) {
                break;
            }
            for (rank, insert) in changes {
                if insert {
                    active.insert(rank);
                } else {
                    active.remove(&rank);
                }
            }
            segments.insert(port as u16, active.first().copied());
        }
        Self(segments)
    }

    fn find(&self, port: u16) -> Option<usize> {
        self.0
            .range(..=port)
            .next_back()
            .and_then(|(_, rank)| *rank)
    }
}
