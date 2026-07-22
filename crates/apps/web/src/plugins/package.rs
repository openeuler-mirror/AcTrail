use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use config_core::daemon::PluginDiscoveryConfig;
use plugin_system::{PluginCapability, PluginManifest, PluginPurpose, PluginRuntimeKind};

pub(super) struct PluginDirectory {
    root: PathBuf,
    max_packages: usize,
    manifest_max_bytes: u64,
}

pub(super) struct InstalledPackage {
    pub key: String,
    pub package_path: PathBuf,
    pub manifest_path: Option<PathBuf>,
    pub plugin_config_path: Option<PathBuf>,
    pub plugin_id: Option<String>,
    pub purpose: Option<PluginPurpose>,
    pub runtime: Option<PluginRuntimeKind>,
    pub requested_capabilities: Vec<String>,
    pub automatic_host_grants: Vec<String>,
    pub parameterized_host_grants: Vec<String>,
    pub warnings: Vec<String>,
    pub issue: Option<String>,
}

impl PluginDirectory {
    pub fn new(config: &PluginDiscoveryConfig) -> Result<Self, String> {
        Ok(Self {
            root: config.resolved_directory()?,
            max_packages: usize::try_from(config.max_packages)
                .map_err(|error| format!("plugins.discovery.max_packages overflow: {error}"))?,
            manifest_max_bytes: config.manifest_max_bytes,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn scan(&self) -> Result<Vec<InstalledPackage>, String> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(format!(
                    "scan plugin directory {} failed: {error}",
                    self.root.display()
                ));
            }
        };
        let canonical_root = fs::canonicalize(&self.root).map_err(|error| {
            format!(
                "canonicalize plugin directory {} failed: {error}",
                self.root.display()
            )
        })?;
        let mut package_paths = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!(
                    "read plugin directory entry in {} failed: {error}",
                    self.root.display()
                )
            })?;
            let file_type = entry.file_type().map_err(|error| {
                format!(
                    "read plugin package type {} failed: {error}",
                    entry.path().display()
                )
            })?;
            if file_type.is_dir() {
                package_paths.push(entry.path());
            }
        }
        package_paths.sort();
        if package_paths.len() > self.max_packages {
            return Err(format!(
                "plugin directory {} contains {} packages, exceeding configured maximum {}",
                self.root.display(),
                package_paths.len(),
                self.max_packages
            ));
        }
        Ok(package_paths
            .into_iter()
            .map(|path| InstalledPackage::inspect(&canonical_root, path, self.manifest_max_bytes))
            .collect())
    }
}

impl InstalledPackage {
    fn inspect(canonical_root: &Path, package_path: PathBuf, manifest_max_bytes: u64) -> Self {
        let key = package_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| package_path.display().to_string());
        let inspected = Self::inspect_valid(
            canonical_root,
            key.clone(),
            package_path.clone(),
            manifest_max_bytes,
        );
        match inspected {
            Ok(package) => package,
            Err(issue) => Self {
                key,
                package_path,
                manifest_path: None,
                plugin_config_path: None,
                plugin_id: None,
                purpose: None,
                runtime: None,
                requested_capabilities: Vec::new(),
                automatic_host_grants: Vec::new(),
                parameterized_host_grants: Vec::new(),
                warnings: Vec::new(),
                issue: Some(issue),
            },
        }
    }

    fn inspect_valid(
        canonical_root: &Path,
        key: String,
        package_path: PathBuf,
        manifest_max_bytes: u64,
    ) -> Result<Self, String> {
        Self::validate_key(&key)?;
        let canonical_package = fs::canonicalize(&package_path).map_err(|error| {
            format!(
                "canonicalize plugin package {} failed: {error}",
                package_path.display()
            )
        })?;
        if !canonical_package.starts_with(canonical_root) {
            return Err(format!(
                "plugin package {} escapes configured directory {}",
                canonical_package.display(),
                canonical_root.display()
            ));
        }
        let manifest_path = Self::find_manifest(&canonical_package)?;
        let metadata = fs::metadata(&manifest_path).map_err(|error| {
            format!(
                "read plugin manifest metadata {} failed: {error}",
                manifest_path.display()
            )
        })?;
        if metadata.len() > manifest_max_bytes {
            return Err(format!(
                "plugin manifest {} is {} bytes, exceeding configured maximum {}",
                manifest_path.display(),
                metadata.len(),
                manifest_max_bytes
            ));
        }
        let raw = fs::read_to_string(&manifest_path).map_err(|error| {
            format!(
                "read plugin manifest {} failed: {error}",
                manifest_path.display()
            )
        })?;
        let manifest = toml::from_str::<PluginManifest>(&raw).map_err(|error| {
            format!(
                "parse plugin manifest {} failed: {error}",
                manifest_path.display()
            )
        })?;
        let warnings = manifest.validate_loadable().map_err(|error| {
            format!(
                "validate plugin manifest {} failed: {error}",
                manifest_path.display()
            )
        })?;
        Self::validate_manifest_assets(&canonical_package, &manifest)?;
        let plugin_config_path = Self::resolve_plugin_config(&manifest_path, &manifest)?;
        let (automatic_host_grants, parameterized_host_grants, activation_issue) =
            Self::classify_grants(&manifest);
        Ok(Self {
            key,
            package_path: canonical_package,
            manifest_path: Some(manifest_path),
            plugin_config_path,
            plugin_id: Some(manifest.id().to_string()),
            purpose: Some(manifest.role()),
            runtime: Some(manifest.runtime_kind()),
            requested_capabilities: manifest
                .capabilities()
                .iter()
                .map(|capability| capability.as_str().to_string())
                .collect(),
            automatic_host_grants,
            parameterized_host_grants,
            warnings,
            issue: activation_issue,
        })
    }

    fn validate_key(key: &str) -> Result<(), String> {
        if key.is_empty()
            || key
                .chars()
                .any(|character| !(character.is_ascii_alphanumeric() || ".-_".contains(character)))
        {
            return Err(format!(
                "plugin package key {key:?} must contain only ASCII letters, digits, '.', '-', or '_'"
            ));
        }
        Ok(())
    }

    fn find_manifest(package_path: &Path) -> Result<PathBuf, String> {
        let mut manifests = Vec::new();
        for entry in fs::read_dir(package_path).map_err(|error| {
            format!(
                "read plugin package {} failed: {error}",
                package_path.display()
            )
        })? {
            let entry = entry.map_err(|error| {
                format!(
                    "read plugin package {} failed: {error}",
                    package_path.display()
                )
            })?;
            if entry
                .file_type()
                .map_err(|error| {
                    format!(
                        "read plugin package entry {} failed: {error}",
                        entry.path().display()
                    )
                })?
                .is_file()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.ends_with(".plugin.toml"))
            {
                manifests.push(entry.path());
            }
        }
        manifests.sort();
        match manifests.as_slice() {
            [manifest] => fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "canonicalize plugin manifest {} failed: {error}",
                    manifest.display()
                )
            }),
            [] => Err(format!(
                "plugin package {} has no *.plugin.toml manifest",
                package_path.display()
            )),
            _ => Err(format!(
                "plugin package {} has multiple *.plugin.toml manifests",
                package_path.display()
            )),
        }
    }

    fn validate_manifest_assets(
        package_path: &Path,
        manifest: &PluginManifest,
    ) -> Result<(), String> {
        if let Some(wasm) = manifest.selected_wasm()
            && let Some(path) = wasm.artifact_path.as_deref()
        {
            Self::validate_asset(package_path, "runtime.wasm.artifact_path", path)?;
        }
        if let Some(path) = manifest.plugin_config.schema_ref.as_deref() {
            Self::validate_asset(package_path, "plugin_config.schema_ref", path)?;
        }
        for (key, definition) in manifest.alert_outputs() {
            Self::validate_asset(
                package_path,
                &format!("outputs.alerts.{key}.payload_schema_ref"),
                &definition.payload_schema_ref,
            )?;
        }
        Ok(())
    }

    fn validate_asset(package_path: &Path, field: &str, raw: &str) -> Result<PathBuf, String> {
        let relative = Path::new(raw);
        if relative.is_absolute() {
            return Err(format!(
                "{field} must be relative for a discovered plugin package"
            ));
        }
        let path = fs::canonicalize(package_path.join(relative)).map_err(|error| {
            format!(
                "resolve {field} {} in package {} failed: {error}",
                raw,
                package_path.display()
            )
        })?;
        if !path.starts_with(package_path) {
            return Err(format!(
                "{field} {raw} escapes package {}",
                package_path.display()
            ));
        }
        if !path.is_file() {
            return Err(format!("{field} {} is not a regular file", path.display()));
        }
        Ok(path)
    }

    fn resolve_plugin_config(
        manifest_path: &Path,
        manifest: &PluginManifest,
    ) -> Result<Option<PathBuf>, String> {
        if !matches!(manifest.plugin_config.format.as_str(), "json" | "toml") {
            return Err(format!(
                "plugin {} config format {} is unsupported for directory discovery",
                manifest.id(),
                manifest.plugin_config.format
            ));
        }
        let name = manifest_path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".plugin.toml"))
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                format!(
                    "plugin manifest {} has no package basename",
                    manifest_path.display()
                )
            })?;
        let path = manifest_path
            .with_file_name(format!("{name}.config.{}", manifest.plugin_config.format));
        if path.is_file() {
            return fs::canonicalize(&path).map(Some).map_err(|error| {
                format!(
                    "canonicalize plugin config {} failed: {error}",
                    path.display()
                )
            });
        }
        if manifest.plugin_config.required {
            return Err(format!(
                "plugin {} requires discovered config {}",
                manifest.id(),
                path.display()
            ));
        }
        Ok(None)
    }

    fn classify_grants(manifest: &PluginManifest) -> (Vec<String>, Vec<String>, Option<String>) {
        let mut grants = Vec::new();
        let mut parameterized = Vec::new();
        let mut unsupported = Vec::new();
        for capability in manifest.capabilities() {
            match capability {
                PluginCapability::EnvRead | PluginCapability::FilePolicyRulesApply => {
                    parameterized.push(capability.as_str());
                }
                PluginCapability::NetworkEgress => unsupported.push(capability.as_str()),
                capability => grants.push(capability.as_str().to_string()),
            }
        }
        let issue = (!unsupported.is_empty()).then(|| {
            format!(
                "plugin {} requests host capabilities without a supported grant format: {}",
                manifest.id(),
                unsupported.join(", ")
            )
        });
        (
            grants,
            parameterized.into_iter().map(str::to_string).collect(),
            issue,
        )
    }

    pub fn activation_ready(&self) -> bool {
        self.issue.is_none() && self.manifest_path.is_some() && self.plugin_id.is_some()
    }
}
