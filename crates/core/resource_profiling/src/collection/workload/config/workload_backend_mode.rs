use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkloadBackendMode {
    #[default]
    Auto,
    Procfs,
    Cgroup,
    Noop,
}
