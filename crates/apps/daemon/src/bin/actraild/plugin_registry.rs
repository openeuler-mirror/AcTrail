//! Persistent plugin instance registry for the `actraild plugin` CLI.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use control_contract::command::PluginLoadCommand;
use model_core::ids::RequestId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
struct PersistentPluginRegistry {
    #[serde(default)]
    plugin_instances: Vec<PersistentPluginInstance>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PersistentPluginInstance {
    instance: String,
    manifest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plugin_config: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    host_grants: Vec<String>,
    enabled: bool,
}

pub fn startup_load_commands(config_path: &Path) -> Result<Vec<PluginLoadCommand>, String> {
    let registry = read_registry(&registry_path(config_path)?)?;
    registry
        .plugin_instances
        .into_iter()
        .filter(|instance| instance.enabled)
        .enumerate()
        .map(|(index, instance)| {
            let request_id = u64::try_from(index + 1)
                .map_err(|error| format!("persistent plugin index overflow: {error}"))?;
            Ok(PluginLoadCommand {
                request_id: RequestId::new(request_id),
                manifest_path: instance.manifest,
                plugin_config_path: instance.plugin_config,
                instance_id: instance.instance,
                host_grants: instance.host_grants,
            })
        })
        .collect()
}

pub fn persist_instance(
    config_path: &Path,
    manifest_path: &Path,
    plugin_config_path: Option<&Path>,
    instance_id: &str,
    host_grants: &[String],
) -> Result<PathBuf, String> {
    if instance_id.trim().is_empty() {
        return Err("persistent plugin instance id must not be empty".to_string());
    }
    let path = registry_path(config_path)?;
    let mut registry = read_registry(&path)?;
    let record = PersistentPluginInstance {
        instance: instance_id.to_string(),
        manifest: canonical_reference(manifest_path, "plugin manifest")?,
        plugin_config: plugin_config_path
            .map(|path| canonical_reference(path, "plugin config"))
            .transpose()?,
        host_grants: host_grants.to_vec(),
        enabled: true,
    };
    registry
        .plugin_instances
        .retain(|existing| existing.instance != record.instance);
    registry.plugin_instances.push(record);
    registry
        .plugin_instances
        .sort_by(|left, right| left.instance.cmp(&right.instance));
    write_registry(&path, &registry)?;
    Ok(path)
}

pub fn remove_instance(config_path: &Path, instance_id: &str) -> Result<PathBuf, String> {
    if instance_id.trim().is_empty() {
        return Err("persistent plugin instance id must not be empty".to_string());
    }
    let path = registry_path(config_path)?;
    let mut registry = read_registry(&path)?;
    registry
        .plugin_instances
        .retain(|existing| existing.instance != instance_id);
    write_registry(&path, &registry)?;
    Ok(path)
}

pub(crate) fn registry_path(config_path: &Path) -> Result<PathBuf, String> {
    let config_path = fs::canonicalize(config_path)
        .map_err(|error| format!("canonicalize config {}: {error}", config_path.display()))?;
    let file_name = config_path
        .file_name()
        .ok_or_else(|| format!("config path {} has no file name", config_path.display()))?
        .to_string_lossy();
    Ok(config_path.with_file_name(format!("{file_name}.plugins.toml")))
}

fn read_registry(path: &Path) -> Result<PersistentPluginRegistry, String> {
    match fs::read_to_string(path) {
        Ok(raw) => toml::from_str::<PersistentPluginRegistry>(&raw).map_err(|error| {
            format!(
                "parse persistent plugin registry {}: {error}",
                path.display()
            )
        }),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            Ok(PersistentPluginRegistry::default())
        }
        Err(error) => Err(format!(
            "read persistent plugin registry {}: {error}",
            path.display()
        )),
    }
}

fn write_registry(path: &Path, registry: &PersistentPluginRegistry) -> Result<(), String> {
    if registry.plugin_instances.is_empty() {
        return match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "remove persistent plugin registry {}: {error}",
                path.display()
            )),
        };
    }
    let raw = toml::to_string_pretty(registry).map_err(|error| {
        format!(
            "serialize persistent plugin registry {}: {error}",
            path.display()
        )
    })?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create persistent plugin registry directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let temp_path = temporary_registry_path(path)?;
    fs::write(&temp_path, raw).map_err(|error| {
        format!(
            "write persistent plugin registry temp {}: {error}",
            temp_path.display()
        )
    })?;
    fs::rename(&temp_path, path).map_err(|error| {
        format!(
            "replace persistent plugin registry {} with {}: {error}",
            path.display(),
            temp_path.display()
        )
    })
}

fn temporary_registry_path(path: &Path) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("registry path {} has no file name", path.display()))?
        .to_string_lossy();
    Ok(path.with_file_name(format!("{file_name}.tmp.{}", std::process::id())))
}

fn canonical_reference(path: &Path, label: &str) -> Result<String, String> {
    fs::canonicalize(path)
        .map(|path| path.display().to_string())
        .map_err(|error| format!("canonicalize {label} {}: {error}", path.display()))
}
