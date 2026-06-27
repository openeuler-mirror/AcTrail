#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginInstanceId(String);

impl PluginInstanceId {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err("plugin instance id must not be empty".to_string());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuiltinPluginInstance {
    id: PluginInstanceId,
    plugin_id: String,
}

impl BuiltinPluginInstance {
    pub fn new(id: PluginInstanceId, plugin_id: impl Into<String>) -> Result<Self, String> {
        let plugin_id = plugin_id.into();
        if plugin_id.trim().is_empty() {
            return Err("builtin plugin id must not be empty".to_string());
        }
        Ok(Self { id, plugin_id })
    }

    pub fn id(&self) -> &PluginInstanceId {
        &self.id
    }

    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }
}
