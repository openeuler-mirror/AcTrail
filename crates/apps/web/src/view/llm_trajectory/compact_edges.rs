use std::collections::{BTreeMap, BTreeSet};

use super::GraphEdge;
use crate::view::actions::ContextCompactRequests;

impl GraphEdge {
    /// Project the already validated compact relations onto retained graph nodes.
    pub(super) fn compact_relations(
        requests: &ContextCompactRequests,
        node_ids: &BTreeSet<&str>,
        partial: &mut bool,
    ) -> Vec<Self> {
        let implementations = requests
            .implementations
            .iter()
            .map(|(request, compact)| (compact.as_str(), request.as_str()))
            .collect::<BTreeMap<_, _>>();
        let mut edges = requests
            .sources
            .iter()
            .filter_map(|(implementation, source)| {
                let Some(compact) = requests.implementations.get(implementation) else {
                    *partial = true;
                    return None;
                };
                Self::compact_edge(
                    source,
                    implementation,
                    "compaction-source",
                    compact,
                    node_ids,
                    partial,
                )
            })
            .collect::<Vec<_>>();
        edges.extend(requests.adoptions.iter().filter_map(|(target, compact)| {
            let Some(implementation) = implementations.get(compact.as_str()) else {
                *partial = true;
                return None;
            };
            Self::compact_edge(
                implementation,
                target,
                "compaction",
                compact,
                node_ids,
                partial,
            )
        }));
        edges
    }

    fn compact_edge(
        source: &str,
        target: &str,
        kind: &'static str,
        compact: &str,
        node_ids: &BTreeSet<&str>,
        partial: &mut bool,
    ) -> Option<Self> {
        if !node_ids.contains(source) || !node_ids.contains(target) {
            *partial = true;
            return None;
        }
        Some(Self {
            source: source.to_string(),
            target: target.to_string(),
            kind,
            confidence: "confirmed",
            context_compact_action_id: Some(compact.to_string()),
        })
    }
}
