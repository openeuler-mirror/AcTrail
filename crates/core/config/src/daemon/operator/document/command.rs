use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct CommandControlDocument {
    pub enabled: bool,
    pub rules_path: String,
}

impl Default for CommandControlDocument {
    fn default() -> Self {
        let config = CommandControlConfig::default();
        Self {
            enabled: config.enabled,
            rules_path: config.rules_path.display().to_string(),
        }
    }
}

impl CommandControlDocument {
    pub(super) fn to_config(&self) -> CommandControlConfig {
        CommandControlConfig {
            enabled: self.enabled,
            rules_path: PathBuf::from(&self.rules_path),
        }
    }
}
