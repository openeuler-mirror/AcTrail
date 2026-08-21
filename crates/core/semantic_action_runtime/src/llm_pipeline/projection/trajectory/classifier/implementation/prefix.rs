use std::collections::{BTreeSet, HashMap};

use crate::llm_pipeline::projection::retention::HistoryAtom;

pub(super) struct PrefixNode {
    parent: Option<(usize, HistoryAtom)>,
    children: HashMap<HistoryAtom, usize>,
    pub(super) candidates: BTreeSet<u64>,
}

pub(super) struct PrefixTrie {
    nodes: Vec<Option<PrefixNode>>,
    free: Vec<usize>,
    non_root_nodes: usize,
}

impl PrefixTrie {
    pub(super) const ROOT: usize = 0;

    pub(super) fn path(&self, history: &[HistoryAtom]) -> Vec<usize> {
        let mut node_id = Self::ROOT;
        let mut path = Vec::with_capacity(history.len());
        for atom in history {
            let Some(next) = self
                .node(node_id)
                .and_then(|node| node.children.get(atom))
                .copied()
            else {
                break;
            };
            path.push(next);
            node_id = next;
        }
        path
    }

    pub(super) fn missing_nodes(&self, history: &[HistoryAtom]) -> usize {
        history.len().saturating_sub(self.path(history).len())
    }

    pub(super) fn ensure_path(&mut self, history: &[HistoryAtom]) -> Option<usize> {
        let mut node_id = Self::ROOT;
        for atom in history {
            if let Some(existing) = self
                .node(node_id)
                .and_then(|node| node.children.get(atom))
                .copied()
            {
                node_id = existing;
                continue;
            }
            let child_id = self.insert_node(PrefixNode {
                parent: Some((node_id, atom.clone())),
                children: HashMap::new(),
                candidates: BTreeSet::new(),
            });
            let Some(parent) = self.node_mut(node_id) else {
                self.nodes[child_id] = None;
                self.free.push(child_id);
                self.non_root_nodes = self.non_root_nodes.saturating_sub(1);
                return None;
            };
            parent.children.insert(atom.clone(), child_id);
            node_id = child_id;
        }
        Some(node_id)
    }

    fn insert_node(&mut self, node: PrefixNode) -> usize {
        self.non_root_nodes += 1;
        if let Some(node_id) = self.free.pop() {
            self.nodes[node_id] = Some(node);
            node_id
        } else {
            self.nodes.push(Some(node));
            self.nodes.len() - 1
        }
    }

    pub(super) fn prune(&mut self, mut node_id: usize) {
        while node_id != Self::ROOT {
            let Some(node) = self.node(node_id) else {
                break;
            };
            if !node.children.is_empty() || !node.candidates.is_empty() {
                break;
            }
            let Some((parent_id, atom)) = node.parent.clone() else {
                break;
            };
            self.nodes[node_id] = None;
            self.free.push(node_id);
            self.non_root_nodes = self.non_root_nodes.saturating_sub(1);
            if let Some(parent) = self.node_mut(parent_id) {
                parent.children.remove(&atom);
            }
            node_id = parent_id;
        }
    }

    pub(super) fn non_root_node_count(&self) -> usize {
        self.non_root_nodes
    }

    pub(super) fn node(&self, node_id: usize) -> Option<&PrefixNode> {
        self.nodes.get(node_id)?.as_ref()
    }

    pub(super) fn node_mut(&mut self, node_id: usize) -> Option<&mut PrefixNode> {
        self.nodes.get_mut(node_id)?.as_mut()
    }
}

impl Default for PrefixTrie {
    fn default() -> Self {
        Self {
            nodes: vec![Some(PrefixNode {
                parent: None,
                children: HashMap::new(),
                candidates: BTreeSet::new(),
            })],
            free: Vec::new(),
            non_root_nodes: 0,
        }
    }
}
