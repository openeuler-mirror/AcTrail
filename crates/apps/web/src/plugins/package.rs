use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use config_core::daemon::PluginDiscoveryConfig;
use plugin_system::{PluginCapability, PluginManifest, PluginPurpose, PluginRuntimeKind};

pub(super) struct PluginDirectory {
    roots: Vec<PathBuf>,
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

enum ScanCandidate {
    Package { root: PathBuf, path: PathBuf },
    Issue(Box<InstalledPackage>),
}

impl ScanCandidate {
    fn root_issue(index: usize, path: &Path, issue: String) -> Self {
        Self::Issue(Box::new(InstalledPackage::diagnostic(
            path.to_path_buf(),
            format!("root:{index}:{path:?}"),
            issue,
        )))
    }

    fn entry_issue(path: PathBuf, issue: String) -> Self {
        let key = format!("{path:?}");
        Self::Issue(Box::new(InstalledPackage::diagnostic(path, key, issue)))
    }

    fn path(&self) -> &Path {
        match self {
            Self::Package { path, .. } => path,
            Self::Issue(package) => &package.package_path,
        }
    }

    fn inspect(self, manifest_max_bytes: u64) -> InstalledPackage {
        match self {
            Self::Package { root, path } => {
                InstalledPackage::inspect(&root, path, manifest_max_bytes)
            }
            Self::Issue(package) => *package,
        }
    }
}

impl PluginDirectory {
    pub fn new(config: &PluginDiscoveryConfig) -> Result<Self, String> {
        Ok(Self {
            roots: config.resolved_directories()?,
            max_packages: usize::try_from(config.max_packages)
                .map_err(|error| format!("plugins.discovery.max_packages overflow: {error}"))?,
            manifest_max_bytes: config.manifest_max_bytes,
        })
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub fn scan(&self) -> Result<Vec<InstalledPackage>, String> {
        let mut seen_roots = BTreeSet::new();
        let mut candidates = Vec::new();
        let mut package_count = 0;
        for (root_index, root) in self.roots.iter().enumerate() {
            let canonical_root = match fs::canonicalize(root) {
                Ok(root) => root,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => {
                    candidates.push(ScanCandidate::root_issue(
                        root_index,
                        root,
                        format!(
                            "canonicalize plugin directory {} failed: {error}",
                            root.display()
                        ),
                    ));
                    continue;
                }
            };
            if !seen_roots.insert(canonical_root.clone()) {
                continue;
            }
            let entries = match fs::read_dir(&canonical_root) {
                Ok(entries) => entries,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => {
                    candidates.push(ScanCandidate::root_issue(
                        root_index,
                        &canonical_root,
                        format!(
                            "scan plugin directory {} failed: {error}",
                            canonical_root.display()
                        ),
                    ));
                    continue;
                }
            };
            let first_candidate = candidates.len();
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        candidates.push(ScanCandidate::root_issue(
                            root_index,
                            &canonical_root,
                            format!(
                                "read plugin directory entry in {} failed: {error}",
                                canonical_root.display()
                            ),
                        ));
                        // An iterator error has no entry path or reliable progress.
                        // Preserve this root's collected entries and scan the next root.
                        break;
                    }
                };
                let candidate = match entry.file_type() {
                    Ok(kind) if kind.is_dir() => ScanCandidate::Package {
                        root: canonical_root.clone(),
                        path: entry.path(),
                    },
                    Ok(_) => continue,
                    Err(error) => ScanCandidate::entry_issue(
                        entry.path(),
                        format!(
                            "read plugin package type {} failed: {error}",
                            entry.path().display()
                        ),
                    ),
                };
                // Unknown entry types consume the same budget as package candidates.
                package_count += 1;
                if package_count > self.max_packages {
                    return Err(format!(
                        "plugin directories exceed configured maximum {} packages",
                        self.max_packages
                    ));
                }
                candidates.push(candidate);
            }
            candidates[first_candidate..].sort_by(|left, right| left.path().cmp(right.path()));
        }
        let mut seen_plugin_ids = BTreeSet::new();
        let mut packages = Vec::new();
        for candidate in candidates {
            let package = candidate.inspect(self.manifest_max_bytes);
            if package
                .plugin_id
                .as_ref()
                .is_some_and(|id| !seen_plugin_ids.insert(id.clone()))
            {
                continue;
            }
            packages.push(package);
        }
        Ok(packages)
    }
}

impl InstalledPackage {
    fn empty(package_path: PathBuf) -> Self {
        Self {
            key: String::new(),
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
            issue: None,
        }
    }

    fn diagnostic(package_path: PathBuf, key: String, issue: String) -> Self {
        let mut package = Self::empty(package_path);
        package.key = key;
        package.issue = Some(issue);
        package
    }

    fn inspect(canonical_root: &Path, package_path: PathBuf, manifest_max_bytes: u64) -> Self {
        let mut package = Self::empty(package_path);
        if let Err(issue) = package.inspect_manifest(canonical_root, manifest_max_bytes) {
            if package.key.is_empty() {
                // A quoted, losslessly escaped path identifies this diagnostic row.
                // Loadable keys are absolute UTF-8 paths and always start with '/'.
                package.key = format!("{:?}", package.package_path);
            }
            package.issue = Some(issue);
        }
        package
    }

    fn inspect_manifest(
        &mut self,
        canonical_root: &Path,
        manifest_max_bytes: u64,
    ) -> Result<(), String> {
        let canonical_path = fs::canonicalize(&self.package_path).map_err(|error| {
            format!(
                "canonicalize plugin package {} failed: {error}",
                self.package_path.display()
            )
        })?;
        let key = canonical_path
            .to_str()
            .ok_or_else(|| {
                format!(
                    "plugin package path is not UTF-8: {}",
                    self.package_path.display()
                )
            })?
            .to_string();
        self.package_path = canonical_path;
        self.key = key;
        if !self.package_path.starts_with(canonical_root) {
            return Err(format!(
                "plugin package {} escapes configured directory {}",
                self.package_path.display(),
                canonical_root.display()
            ));
        }
        let manifest_path = Self::find_manifest(&self.package_path)?;
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
        // Keep an identified package's priority even if its subsequent checks fail.
        // Unreadable or unparseable manifests never acquire an inferred identity.
        if !manifest.id().trim().is_empty() {
            self.plugin_id = Some(manifest.id().to_string());
        }
        let warnings = manifest.validate_loadable().map_err(|error| {
            format!(
                "validate plugin manifest {} failed: {error}",
                manifest_path.display()
            )
        })?;
        Self::validate_manifest_assets(&self.package_path, &manifest)?;
        let plugin_config_path = Self::resolve_plugin_config(&manifest_path, &manifest)?;
        let (automatic_host_grants, parameterized_host_grants, activation_issue) =
            Self::classify_grants(&manifest);
        self.manifest_path = Some(manifest_path);
        self.plugin_config_path = plugin_config_path;
        self.purpose = Some(manifest.role());
        self.runtime = Some(manifest.runtime_kind());
        self.requested_capabilities = manifest
            .capabilities()
            .iter()
            .map(|capability| capability.as_str().to_string())
            .collect();
        self.automatic_host_grants = automatic_host_grants;
        self.parameterized_host_grants = parameterized_host_grants;
        self.warnings = warnings;
        self.issue = activation_issue;
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
                PluginCapability::EnvRead
                | PluginCapability::FilePolicyRulesApply
                | PluginCapability::CommandPolicyRulesApply
                | PluginCapability::NetworkPolicyRulesApply => {
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
