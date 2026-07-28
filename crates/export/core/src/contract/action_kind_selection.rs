use std::collections::BTreeMap;

use semantic_action::{SemanticActionKind, SemanticActionKind::FileTtyIo};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionKindSelection {
    default_enabled: bool,
    overrides: BTreeMap<SemanticActionKind, bool>,
}

impl SemanticActionKindSelection {
    pub fn from_config_entries(
        entries: impl IntoIterator<Item = (String, bool)>,
    ) -> Result<Self, String> {
        let mut default_enabled = false;
        let mut default_seen = false;
        let mut overrides = BTreeMap::new();

        for (key, enabled) in entries {
            if key == "default" {
                if default_seen {
                    return Err("duplicate action kind selection key default".to_string());
                }
                default_seen = true;
                default_enabled = enabled;
                continue;
            }

            let kind = SemanticActionKind::parse(&key)
                .ok_or_else(|| format!("unknown action kind selection key {key}"))?;
            if kind == FileTtyIo {
                return Err(
                    "action kind file.tty_io is filtered before observation export and is not configurable"
                        .to_string(),
                );
            }
            if overrides.insert(kind, enabled).is_some() {
                return Err(format!("duplicate action kind selection key {key}"));
            }
        }

        Ok(Self {
            default_enabled,
            overrides,
        })
    }

    pub fn enabled(&self, kind: SemanticActionKind) -> bool {
        self.overrides
            .get(&kind)
            .copied()
            .unwrap_or(self.default_enabled)
    }

    pub const fn default_enabled(&self) -> bool {
        self.default_enabled
    }

    pub fn overrides(&self) -> &BTreeMap<SemanticActionKind, bool> {
        &self.overrides
    }
}
