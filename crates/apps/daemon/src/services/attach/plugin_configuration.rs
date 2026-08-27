//! Configuration bridge for loaded and builtin plugins.

use control_contract::reply::{ControlError, PluginConfigReply, PluginConfigValidationReply};

use crate::services::alert_forwarding::{ALERT_FORWARDING_INSTANCE_ID, ALERT_FORWARDING_PLUGIN_ID};

use super::StorageAttachService;

impl StorageAttachService {
    pub(super) fn plugin_config_impl(
        &self,
        instance_id: &str,
    ) -> Result<PluginConfigReply, ControlError> {
        if instance_id == ALERT_FORWARDING_INSTANCE_ID {
            return Ok(PluginConfigReply {
                instance_id: ALERT_FORWARDING_INSTANCE_ID.to_string(),
                plugin_id: ALERT_FORWARDING_PLUGIN_ID.to_string(),
                editable: true,
                config_json: self.alert_forwarding.config_json()?,
                schema_json: self.alert_forwarding.schema_json().to_string(),
            });
        }
        if !self.plugin_configs.runtime_managed(instance_id)? {
            return self.plugin_configs.document(instance_id);
        }
        let current = self
            .control_plugins
            .runtime_config(instance_id)
            .map_err(|error| ControlError::new(error.code, error.message))?;
        self.plugin_configs
            .runtime_document(instance_id, &current.config_json)
    }

    pub(super) fn validate_plugin_config_impl(
        &self,
        instance_id: &str,
        config_json: &str,
    ) -> Result<PluginConfigValidationReply, ControlError> {
        if instance_id == ALERT_FORWARDING_INSTANCE_ID {
            let error = self
                .alert_forwarding
                .validate_config(config_json)
                .err()
                .map(|error| error.message);
            return Ok(PluginConfigValidationReply {
                instance_id: ALERT_FORWARDING_INSTANCE_ID.to_string(),
                valid: error.is_none(),
                errors: error.into_iter().collect(),
            });
        }
        if !self.plugin_configs.runtime_managed(instance_id)? {
            let mut validation = self.plugin_configs.validate(instance_id, config_json)?;
            if validation.valid {
                let update = self
                    .plugin_configs
                    .prepare_update(instance_id, config_json)?;
                if let Err(error) = self.precheck_plugin_config(&update) {
                    validation.errors = vec![error.message];
                    validation.valid = false;
                }
            }
            return Ok(validation);
        }
        let current = self
            .control_plugins
            .runtime_config(instance_id)
            .map_err(|error| ControlError::new(error.code, error.message))?;
        let mut validation =
            self.plugin_configs
                .validate_runtime(instance_id, &current.config_json, config_json)?;
        if validation.valid {
            validation.errors = self
                .control_plugins
                .validate_runtime_config(instance_id, config_json)
                .map_err(|error| ControlError::new(error.code, error.message))?;
            validation.valid = validation.errors.is_empty();
        }
        Ok(validation)
    }

    pub(super) fn update_plugin_config_impl(
        &mut self,
        instance_id: &str,
        config_json: &str,
    ) -> Result<PluginConfigReply, ControlError> {
        if instance_id == ALERT_FORWARDING_INSTANCE_ID {
            self.alert_forwarding.update_config(config_json)?;
            return self.plugin_config_impl(instance_id);
        }
        if self.plugin_configs.runtime_managed(instance_id)? {
            return self.update_runtime_plugin_config(instance_id, config_json);
        }
        let update = self
            .plugin_configs
            .prepare_update(instance_id, config_json)?;
        self.precheck_plugin_config(&update)?;
        self.remove_plugin_runtime_impl(instance_id)?;
        self.plugin_configs.remove(instance_id);
        self.install_updated_plugin_impl(update)?;
        self.plugin_configs.document(instance_id)
    }

    fn update_runtime_plugin_config(
        &mut self,
        instance_id: &str,
        config_json: &str,
    ) -> Result<PluginConfigReply, ControlError> {
        let current = self
            .control_plugins
            .runtime_config(instance_id)
            .map_err(|error| ControlError::new(error.code, error.message))?;
        let update = self.plugin_configs.prepare_runtime_update(
            instance_id,
            &current.config_json,
            config_json,
        )?;
        let canonical_json = update.raw.as_deref().ok_or_else(|| {
            ControlError::new(
                "plugin_config",
                "runtime config update has no JSON document",
            )
        })?;
        let errors = self
            .control_plugins
            .validate_runtime_config(instance_id, canonical_json)
            .map_err(|error| ControlError::new(error.code, error.message))?;
        if !errors.is_empty() {
            return Err(ControlError::new(
                "plugin_config_validation",
                errors.join("; "),
            ));
        }
        self.control_plugins
            .submit_runtime_config(instance_id, canonical_json)
            .map_err(|error| ControlError::new(error.code, error.message))?;
        let current = self
            .control_plugins
            .runtime_config(instance_id)
            .map_err(|error| ControlError::new(error.code, error.message))?;
        self.plugin_configs
            .commit_runtime_config(instance_id, &current.config_json)?;
        self.plugin_configs
            .runtime_document(instance_id, &current.config_json)
    }
}
