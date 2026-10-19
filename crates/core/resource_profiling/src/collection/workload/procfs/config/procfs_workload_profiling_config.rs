use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProcfsWorkloadProfilingConfig {
    include_children: bool,
}

impl Default for ProcfsWorkloadProfilingConfig {
    fn default() -> Self {
        Self::new(super::constants::INCLUDE_CHILDREN)
    }
}

impl ProcfsWorkloadProfilingConfig {
    pub fn parse(table: &toml::Table) -> Result<Self, String> {
        toml::Value::Table(table.clone())
            .try_into()
            .map_err(|error| format!("resource_profiling.collection.workload.procfs: {error}"))
    }
    pub fn new(include_children: bool) -> Self {
        Self { include_children }
    }

    pub fn include_children(&self) -> bool {
        self.include_children
    }
}
