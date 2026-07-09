//! WASM runtime adapter for AcTrail plugin-system consumers.

mod codec;
mod component_control;
mod component_observation;
mod control;
mod engine;
mod host;
mod memory;
mod observation;

pub use codec::{WasmLlmCodecPlugin, build_wasm_llm_codec_plugin};
pub use control::{WasmControlDecider, build_wasm_control_decider};
pub use observation::{WasmObservationConsumer, build_wasm_observation_consumer};
