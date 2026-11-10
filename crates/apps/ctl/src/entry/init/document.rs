use std::fs;
use std::path::{Path, PathBuf};

use config_core::daemon::OperatorConfig;
use config_export_filter::ConfigExportFilter;
use model_core::capability::Capability;
use toml::Value;

use crate::args::InitMode;

#[path = "file.rs"]
mod file;

pub(super) struct DeliveryConfigDocument {
    value: Value,
}

impl DeliveryConfigDocument {
    pub(super) fn defaults(filter: bool) -> Result<Self, String> {
        let template = OperatorConfig::default_hierarchical_template()?;
        let defaults = toml::from_str(&template)
            .map_err(|error| format!("parse default operator config: {error}"))?;
        let value = if filter {
            ConfigExportFilter::bundled(&defaults)?.project(&defaults)?
        } else {
            defaults
        };
        Ok(Self { value })
    }

    pub(super) fn apply_mode(&mut self, mode: InitMode) -> Result<(), String> {
        let preset = match mode {
            InitMode::Complete => include_str!("../complete.toml"),
            InitMode::Profile => {
                self.apply_profile_capabilities()?;
                include_str!("../profile.toml")
            }
        };
        self.patch(preset, "mode preset")
    }

    pub(super) fn patch_files(&mut self, patch_paths: &[PathBuf]) -> Result<(), String> {
        for path in patch_paths {
            let raw = fs::read_to_string(path)
                .map_err(|error| format!("read config patch {}: {error}", path.display()))?;
            self.patch(&raw, &format!("config patch {}", path.display()))?;
        }
        Ok(())
    }

    pub(super) fn write(&self, path: &Path, overwrite: bool) -> Result<(), String> {
        let raw = toml::to_string_pretty(&self.value)
            .map_err(|error| format!("render delivery operator config: {error}"))?;
        OperatorConfig::parse(&raw)
            .map_err(|error| format!("validate delivery operator config: {error}"))?;
        file::ConfigOutput::new(path, overwrite).write(&raw)
    }

    fn apply_profile_capabilities(&mut self) -> Result<(), String> {
        let mut config = OperatorConfig::init()?;
        config.capture_profile.capabilities.retain(|request| {
            !matches!(
                request.capability,
                Capability::EnforcementFilePermissionFanotify
                    | Capability::EnforcementFileMkdirSeccomp
                    | Capability::EnforcementFileRmdirSeccomp
                    | Capability::EnforcementCommandExecutionSeccomp
                    | Capability::EnforcementNetworkConnectSeccomp
                    | Capability::EnforcementFilePermissionLsm
                    | Capability::EnforcementCommandExecutionLsm
                    | Capability::EnforcementNetworkConnectLsm
            )
        });
        let full: Value = toml::from_str(&config.dump()?)
            .map_err(|error| format!("parse profile capabilities: {error}"))?;
        let mut capture = toml::Table::new();
        for key in [
            "capabilities",
            "opportunistic_capabilities",
            "disabled_capabilities",
        ] {
            let value = full
                .get("capture")
                .and_then(|capture| capture.get(key))
                .ok_or_else(|| format!("profile config is missing capture.{key}"))?;
            capture.insert(key.to_string(), value.clone());
        }
        let mut preset = toml::Table::new();
        preset.insert("capture".to_string(), Value::Table(capture));
        Self::merge(&mut self.value, Value::Table(preset));
        Ok(())
    }

    fn patch(&mut self, raw: &str, source: &str) -> Result<(), String> {
        let patch = toml::from_str(raw).map_err(|error| format!("parse {source}: {error}"))?;
        Self::merge(&mut self.value, patch);
        Ok(())
    }

    fn merge(base: &mut Value, patch: Value) {
        match (base, patch) {
            (Value::Table(base), Value::Table(patch)) => {
                for (key, patch_value) in patch {
                    if let Some(base_value) = base.get_mut(&key) {
                        Self::merge(base_value, patch_value);
                    } else {
                        base.insert(key, patch_value);
                    }
                }
            }
            (base, patch) => *base = patch,
        }
    }
}
