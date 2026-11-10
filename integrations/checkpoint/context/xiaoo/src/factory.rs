//! Exports the shared creation entry and constructs configured Xiaoo backends.

use abi_stable::{
    export_root_module, prefix_type::PrefixTypeTrait, sabi_extern_fn,
    sabi_trait::prelude::TD_Opaque, std_types::RResult,
};
use checkpoint::{
    BackendConfig, BackendError, BackendKind, CheckpointBackend, CheckpointModule,
    CheckpointModuleRef,
};

use crate::{backend::XiaooBackend, config::Config};

struct Factory;

impl Factory {
    #[sabi_extern_fn]
    extern "C" fn create(config: BackendConfig) -> RResult<CheckpointBackend, BackendError> {
        Config::from_json(config.json.as_str())
            .map(|config| CheckpointBackend::from_value(XiaooBackend::new(config), TD_Opaque))
            .map_err(BackendError::from)
            .into()
    }
}

#[export_root_module]
pub fn checkpoint_module() -> CheckpointModuleRef {
    CheckpointModule {
        create: Factory::create,
        kind: BackendKind::Context,
    }
    .leak_into_prefix()
}
