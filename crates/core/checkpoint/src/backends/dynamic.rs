//! An integration loaded from its dynamic library, which creates backend instances.

use std::{fs, path::Path};

use abi_stable::library::lib_header_from_path;
use checkpoint::{BackendConfig, BackendKind, CheckpointBackend, CheckpointModuleRef};

pub(crate) struct DynamicIntegration {
    module: CheckpointModuleRef,
    /// Daemon-owned runtime settings. Only the integration interprets these bytes.
    runtime_json: String,
}

impl DynamicIntegration {
    /// Goes through the library header, since `RootModule::load_from` caches one library per
    /// module type. The library stays loaded for the life of the process.
    pub(crate) fn load(library: &Path, config: &Path) -> Result<Self, String> {
        let module = lib_header_from_path(library)
            .and_then(|header| header.init_root_module::<CheckpointModuleRef>())
            .map_err(|error| format!("{}: {error}", library.display()))?;
        let runtime_json =
            fs::read_to_string(config).map_err(|error| format!("{}: {error}", config.display()))?;
        Ok(Self {
            module,
            runtime_json,
        })
    }

    pub(crate) fn kind(&self) -> BackendKind {
        self.module.kind()
    }

    /// A new instance, which still has to connect.
    pub(crate) fn create(
        &self,
        trace_config_path: Option<&str>,
    ) -> Result<CheckpointBackend, String> {
        let create = self.module.create();
        create(BackendConfig {
            runtime_json: self.runtime_json.as_str().into(),
            trace_config_path: trace_config_path.map(Into::into).into(),
        })
        .into_result()
        .map_err(|error| error.message.into_string())
    }
}
