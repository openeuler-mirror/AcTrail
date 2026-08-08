//! Runtime-managed WIT component configuration calls and result parsing.

use std::time::Instant;

use plugin_system::PluginRuntimeError;
use wasmtime::AsContextMut;
use wasmtime::component::{Func, Val};

use crate::control::{arm_epoch_timeout, call_timeout_error, disarm_epoch_timeout};
use crate::engine::{WasmStore, reset_epoch_deadline_unbounded, reset_fuel};

use super::WitComponentControlState;
use super::component_abi;

#[derive(Clone)]
pub(super) struct RuntimeConfigFunctions {
    get: Func,
    validate: Func,
    submit: Func,
}

impl RuntimeConfigFunctions {
    pub(super) fn find(
        instance: &wasmtime::component::Instance,
        store: &mut WasmStore,
    ) -> Result<Option<Self>, PluginRuntimeError> {
        let Some(interface) = instance.get_export_index(
            store.as_context_mut(),
            None,
            component_abi::RUNTIME_CONFIG_EXPORT,
        ) else {
            return Ok(None);
        };
        let mut required = |name: &str| {
            instance
                .get_export_index(store.as_context_mut(), Some(&interface), name)
                .and_then(|export| instance.get_func(store.as_context_mut(), &export))
                .ok_or_else(|| {
                    PluginRuntimeError::new(
                        "wasm_runtime",
                        format!("runtime-config export missing function {name}"),
                    )
                })
        };
        Ok(Some(Self {
            get: required(component_abi::RUNTIME_CONFIG_GET_EXPORT)?,
            validate: required(component_abi::RUNTIME_CONFIG_VALIDATE_EXPORT)?,
            submit: required(component_abi::RUNTIME_CONFIG_SUBMIT_EXPORT)?,
        }))
    }

    pub(super) fn get(
        &self,
        state: &mut WitComponentControlState,
    ) -> Result<String, PluginRuntimeError> {
        let result = self.call(state, &self.get, &[], "get")?;
        let config = parse_runtime_config_string(result, "get")?;
        let max_bytes = state
            .store
            .data()
            .host_limits()
            .plugin_config_read_max_bytes;
        if config.len() > max_bytes {
            return Err(PluginRuntimeError::new(
                "plugin_config",
                format!("runtime config exceeded {max_bytes} bytes"),
            ));
        }
        Ok(config)
    }

    pub(super) fn validate(
        &self,
        state: &mut WitComponentControlState,
        config_json: &str,
    ) -> Result<Vec<String>, PluginRuntimeError> {
        Self::validate_input(state, config_json)?;
        let result = self.call(
            state,
            &self.validate,
            &[Val::String(config_json.to_string())],
            "validate",
        )?;
        let errors = parse_runtime_config_errors(result)?;
        let output_bytes = errors.iter().map(String::len).sum::<usize>();
        let max_bytes = state
            .store
            .data()
            .host_limits()
            .plugin_command_output_max_bytes;
        if output_bytes > max_bytes {
            return Err(PluginRuntimeError::new(
                "plugin_config",
                format!("runtime config validation output exceeded {max_bytes} bytes"),
            ));
        }
        Ok(errors)
    }

    pub(super) fn submit(
        &self,
        state: &mut WitComponentControlState,
        config_json: &str,
    ) -> Result<(), PluginRuntimeError> {
        Self::validate_input(state, config_json)?;
        let result = self.call(
            state,
            &self.submit,
            &[Val::String(config_json.to_string())],
            "submit",
        )?;
        parse_runtime_config_unit(result)
    }

    pub(super) fn validate_input(
        state: &WitComponentControlState,
        config_json: &str,
    ) -> Result<(), PluginRuntimeError> {
        let max_bytes = state
            .store
            .data()
            .host_limits()
            .plugin_config_read_max_bytes;
        if config_json.len() > max_bytes {
            return Err(PluginRuntimeError::new(
                "plugin_config",
                format!("runtime config input exceeded {max_bytes} bytes"),
            ));
        }
        Ok(())
    }

    pub(super) fn call(
        &self,
        state: &mut WitComponentControlState,
        function: &Func,
        params: &[Val],
        operation: &str,
    ) -> Result<Val, PluginRuntimeError> {
        reset_fuel(&mut state.store, state.fuel_per_call)?;
        reset_epoch_deadline_unbounded(&mut state.store);
        let timeout_ms = Some(state.store.data().host_limits().plugin_command_timeout_ms);
        let started_at = Instant::now();
        let generation = state.deadline_generation.clone();
        let deadline = arm_epoch_timeout(
            state.engine.clone(),
            &mut state.store,
            timeout_ms,
            &generation,
        );
        let mut results = [Val::Result(Ok(None))];
        let result = function.call(&mut state.store, params, &mut results);
        disarm_epoch_timeout(&mut state.store, deadline, &generation);
        if let Err(error) = result {
            return Err(call_timeout_error(
                &mut state.store,
                &format!("wasm component runtime-config {operation}"),
                error,
                timeout_ms,
                started_at,
            ));
        }
        function.post_return(&mut state.store).map_err(|error| {
            PluginRuntimeError::new(
                "wasm_runtime",
                format!("runtime-config {operation} post-return failed: {error}"),
            )
        })?;
        Ok(results.into_iter().next().expect("one result slot"))
    }
}

fn runtime_config_result(value: Val, operation: &str) -> Result<Option<Val>, PluginRuntimeError> {
    match value {
        Val::Result(Ok(value)) => Ok(value.map(|value| *value)),
        Val::Result(Err(Some(error))) => {
            let message = match *error {
                Val::String(message) => message,
                other => format!("{other:?}"),
            };
            Err(PluginRuntimeError::new(
                "plugin_config",
                format!("runtime-config {operation} failed: {message}"),
            ))
        }
        Val::Result(Err(None)) => Err(PluginRuntimeError::new(
            "plugin_config",
            format!("runtime-config {operation} failed without an error message"),
        )),
        other => Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("runtime-config {operation} returned invalid result {other:?}"),
        )),
    }
}

fn parse_runtime_config_string(value: Val, operation: &str) -> Result<String, PluginRuntimeError> {
    match runtime_config_result(value, operation)? {
        Some(Val::String(value)) => Ok(value),
        other => Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("runtime-config {operation} returned invalid payload {other:?}"),
        )),
    }
}

fn parse_runtime_config_errors(value: Val) -> Result<Vec<String>, PluginRuntimeError> {
    match runtime_config_result(value, "validate")? {
        Some(Val::List(values)) => values
            .into_iter()
            .map(|value| match value {
                Val::String(value) => Ok(value),
                other => Err(PluginRuntimeError::new(
                    "wasm_runtime",
                    format!("runtime-config validate returned invalid error {other:?}"),
                )),
            })
            .collect(),
        other => Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("runtime-config validate returned invalid payload {other:?}"),
        )),
    }
}

fn parse_runtime_config_unit(value: Val) -> Result<(), PluginRuntimeError> {
    match runtime_config_result(value, "submit")? {
        None => Ok(()),
        other => Err(PluginRuntimeError::new(
            "wasm_runtime",
            format!("runtime-config submit returned invalid payload {other:?}"),
        )),
    }
}
