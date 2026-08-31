//! Insertion-ordered values with direct lookup by correlation identifier.

use std::collections::HashMap;
use std::iter::FusedIterator;

struct Node<T> {
    value: T,
    previous: Option<String>,
    next: Option<String>,
}

/// A keyed queue whose lookup, insertion, and removal operations are average
/// constant time.
pub(in crate::llm_pipeline) struct IndexedQueue<T> {
    nodes: HashMap<String, Node<T>>,
    front: Option<String>,
    back: Option<String>,
}

impl<T> IndexedQueue<T> {
    pub(in crate::llm_pipeline) fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            front: None,
            back: None,
        }
    }

    /// Replaces an existing value in place, or appends a new value.
    pub(in crate::llm_pipeline) fn upsert(&mut self, key: String, value: T) -> Option<T> {
        if let Some(node) = self.nodes.get_mut(&key) {
            return Some(std::mem::replace(&mut node.value, value));
        }

        let previous = self.back.take();
        if let Some(previous_key) = &previous {
            if let Some(previous_node) = self.nodes.get_mut(previous_key) {
                previous_node.next = Some(key.clone());
            }
        } else {
            self.front = Some(key.clone());
        }

        self.back = Some(key.clone());
        self.nodes.insert(
            key,
            Node {
                value,
                previous,
                next: None,
            },
        );
        None
    }

    pub(in crate::llm_pipeline) fn get(&self, key: &str) -> Option<&T> {
        self.nodes.get(key).map(|node| &node.value)
    }

    pub(in crate::llm_pipeline) fn get_mut(&mut self, key: &str) -> Option<&mut T> {
        self.nodes.get_mut(key).map(|node| &mut node.value)
    }

    pub(in crate::llm_pipeline) fn front(&self) -> Option<&T> {
        let key = self.front.as_deref()?;
        self.nodes.get(key).map(|node| &node.value)
    }

    pub(in crate::llm_pipeline) fn back(&self) -> Option<&T> {
        let key = self.back.as_deref()?;
        self.nodes.get(key).map(|node| &node.value)
    }

    pub(in crate::llm_pipeline) fn pop_front(&mut self) -> Option<T> {
        let key = self.front.clone()?;
        self.remove_node(&key).map(|node| node.value)
    }

    pub(in crate::llm_pipeline) fn pop_back(&mut self) -> Option<T> {
        let key = self.back.clone()?;
        self.remove_node(&key).map(|node| node.value)
    }

    /// Inserts at the front. An existing key is replaced and moved there.
    pub(in crate::llm_pipeline) fn push_front(&mut self, key: String, value: T) -> Option<T> {
        let previous_value = self.remove_node(&key).map(|node| node.value);
        let next = self.front.take();
        if let Some(next_key) = &next {
            if let Some(next_node) = self.nodes.get_mut(next_key) {
                next_node.previous = Some(key.clone());
            }
        } else {
            self.back = Some(key.clone());
        }

        self.front = Some(key.clone());
        self.nodes.insert(
            key,
            Node {
                value,
                previous: None,
                next,
            },
        );
        previous_value
    }

    pub(in crate::llm_pipeline) fn remove(&mut self, key: &str) -> Option<T> {
        self.remove_node(key).map(|node| node.value)
    }

    pub(in crate::llm_pipeline) fn len(&self) -> usize {
        self.nodes.len()
    }

    pub(in crate::llm_pipeline) fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub(in crate::llm_pipeline) fn iter(&self) -> Iter<'_, T> {
        Iter {
            nodes: &self.nodes,
            next: self.front.as_deref(),
            remaining: self.nodes.len(),
        }
    }

    pub(in crate::llm_pipeline) fn into_values(self) -> IntoValues<T> {
        IntoValues {
            nodes: self.nodes,
            next: self.front,
        }
    }

    fn remove_node(&mut self, key: &str) -> Option<Node<T>> {
        let node = self.nodes.remove(key)?;

        if let Some(previous_key) = &node.previous {
            if let Some(previous_node) = self.nodes.get_mut(previous_key) {
                previous_node.next.clone_from(&node.next);
            }
        } else {
            self.front.clone_from(&node.next);
        }

        if let Some(next_key) = &node.next {
            if let Some(next_node) = self.nodes.get_mut(next_key) {
                next_node.previous.clone_from(&node.previous);
            }
        } else {
            self.back.clone_from(&node.previous);
        }

        Some(node)
    }
}

impl<T> Default for IndexedQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub(in crate::llm_pipeline) struct Iter<'a, T> {
    nodes: &'a HashMap<String, Node<T>>,
    next: Option<&'a str>,
    remaining: usize,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        let key = self.next.take()?;
        let Some(node) = self.nodes.get(key) else {
            self.remaining = 0;
            return None;
        };
        self.next = node.next.as_deref();
        self.remaining -= 1;
        Some(&node.value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T> ExactSizeIterator for Iter<'_, T> {}
impl<T> FusedIterator for Iter<'_, T> {}

pub(in crate::llm_pipeline) struct IntoValues<T> {
    nodes: HashMap<String, Node<T>>,
    next: Option<String>,
}

impl<T> Iterator for IntoValues<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        let key = self.next.take()?;
        let Some(node) = self.nodes.remove(&key) else {
            self.nodes.clear();
            return None;
        };
        self.next = node.next;
        Some(node.value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.nodes.len();
        (remaining, Some(remaining))
    }
}

impl<T> ExactSizeIterator for IntoValues<T> {}
impl<T> FusedIterator for IntoValues<T> {}
