use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CgroupBackendMode {
    #[default]
    Auto,
    V1,
    V2,
}
