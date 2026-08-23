use std::collections::BTreeSet;

use crate::AlertForwardingConfig;

pub(super) struct CategoryFilter {
    all_categories: bool,
    categories: BTreeSet<String>,
}

impl CategoryFilter {
    pub(super) fn from_config(config: &AlertForwardingConfig) -> Self {
        Self {
            all_categories: config.all_categories(),
            categories: config.categories().iter().cloned().collect(),
        }
    }

    #[inline]
    pub(super) fn accepts(&self, category: &str) -> bool {
        self.all_categories || self.categories.contains(category)
    }
}
