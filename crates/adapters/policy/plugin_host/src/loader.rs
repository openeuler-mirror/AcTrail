//! Versioned plugin loading boundary for policy adapters.

use policy_plugin_contract::abi::PolicyPlugin;

use crate::errors::PluginHostError;
use crate::registry::PluginRegistry;

pub struct PluginLoader {
    expected_api_version: String,
}

impl PluginLoader {
    pub fn new(expected_api_version: impl Into<String>) -> Self {
        Self {
            expected_api_version: expected_api_version.into(),
        }
    }

    pub fn load_into(
        &self,
        registry: &mut PluginRegistry,
        plugin: Box<dyn PolicyPlugin>,
    ) -> Result<(), PluginHostError> {
        if plugin.manifest().api_version != self.expected_api_version {
            return Err(PluginHostError::new(
                "load",
                format!(
                    "plugin {} targets api {} but runtime expects {}",
                    plugin.manifest().name,
                    plugin.manifest().api_version,
                    self.expected_api_version,
                ),
            ));
        }

        registry.register(plugin);
        Ok(())
    }
}
