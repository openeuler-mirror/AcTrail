pub(super) use crate::llm_pipeline::provider::{
    HistoryItem, RequestHistory as ContextHistory, RequestHistoryObservation as HistoryObservation,
    RequestHistorySource,
};

mod source;

pub(super) use source::HistoryMaterializer;

pub(super) struct HistoryContinuationFilter {
    max_items: usize,
    max_bytes: usize,
}

impl HistoryContinuationFilter {
    pub(super) fn new(max_items: usize, max_bytes: usize) -> Self {
        Self {
            max_items,
            max_bytes,
        }
    }

    pub(super) fn snapshot(&self, source: &RequestHistorySource) -> HistoryObservation {
        source.snapshot(self.max_items, self.max_bytes)
    }

    /// Reusing compaction candidate's complete conversational prefix is ordinary continuation,
    /// including a fork that carries compaction candidate's answer into another branch.
    pub(super) fn retains_history(
        &self,
        implementation: &ContextHistory,
        adoption: &ContextHistory,
    ) -> bool {
        let mut implementation_items = implementation
            .items
            .iter()
            .filter(|item| item.is_conversational())
            .peekable();
        if implementation_items.peek().is_none() {
            return false;
        }
        let mut adoption_items = adoption
            .items
            .iter()
            .filter(|item| item.is_conversational());
        implementation_items.all(|old| adoption_items.next().is_some_and(|item| item.retains(old)))
    }

    /// Optional source-to-compaction provenance requires all conversational items from A.
    /// Select one proof shape, without allocating or trying multiple capsules.
    fn consumes_history(&self, source: &ContextHistory, implementation: &ContextHistory) -> bool {
        let mut implementation_items = implementation
            .items
            .iter()
            .filter(|item| item.is_conversational());
        let Some(capsule) = implementation_items.next() else {
            return false;
        };
        if !matches!(capsule.role.as_str(), "user" | "human")
            || implementation_items.next().is_some()
        {
            return self.retains_history(source, implementation);
        }

        let mut source_items = source
            .items
            .iter()
            .filter(|item| item.is_conversational())
            .peekable();
        let Some(first) = source_items.next() else {
            return false;
        };
        if source_items.peek().is_none() && first.role == capsule.role {
            return capsule.retains(first);
        }
        let mut offset = 0;
        for old in std::iter::once(first).chain(source_items) {
            let Some(end) = old.embedded_in(&capsule.text, offset) else {
                return false;
            };
            offset = end;
        }
        true
    }
}

impl HistoryItem {
    fn same_content(&self, other: &Self) -> bool {
        self.canonical == other.canonical
            || (self.tool_calls.is_empty()
                && other.tool_calls.is_empty()
                && !self.text.is_empty()
                && self.text.trim() == other.text.trim())
    }

    fn retains(&self, old: &Self) -> bool {
        self.role == old.role
            && (self.same_content(old)
                || old
                    .canonical
                    .strip_suffix(']')
                    .filter(|prefix| prefix.starts_with('[') && prefix.len() > 1)
                    .is_some_and(|prefix| {
                        self.canonical
                            .strip_prefix(prefix)
                            .is_some_and(|suffix| suffix.starts_with(','))
                    })
                || (self.tool_calls.is_empty() && old.embedded_in(&self.text, 0).is_some()))
    }

    fn embedded_in(&self, carrier: &str, mut offset: usize) -> Option<usize> {
        if self.parts.is_empty() && self.tool_calls.is_empty() {
            return None;
        }
        for range in &self.parts {
            let part = &self.text[range.clone()];
            offset += carrier[offset..].find(part)? + part.len();
        }
        for range in &self.tool_calls {
            let invocation = &self.text[range.clone()];
            let relative =
                carrier[offset..]
                    .match_indices(invocation)
                    .find_map(|(position, _)| {
                        let start = offset + position;
                        let continues_name =
                            carrier[..start].chars().next_back().is_some_and(|c| {
                                c.is_alphanumeric() || matches!(c, '_' | '.' | ':' | '/' | '-')
                            });
                        (!continues_name).then_some(position)
                    })?;
            offset += relative + invocation.len();
        }
        Some(offset)
    }

    fn is_conversational(&self) -> bool {
        matches!(self.role.as_str(), "user" | "human" | "assistant" | "tool")
    }
}
