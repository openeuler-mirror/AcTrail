use crate::probe_detector::contract::detector::DetectorConfigError;

#[derive(Clone, Debug)]
pub(super) struct Limits {
    pub(super) function_instructions: usize,
    pub(super) prefix_instructions: usize,
    pub(super) candidates: usize,
    pub(super) matching_calls: usize,
    pub(super) caller_instructions: usize,
    pub(super) dataflow_steps: usize,
}

impl Limits {
    pub(super) fn from_environment() -> Result<Self, DetectorConfigError> {
        let mut limits = Self {
            function_instructions: 512,
            prefix_instructions: 64,
            candidates: 16,
            matching_calls: 128,
            caller_instructions: 48,
            dataflow_steps: 32768,
        };
        for (suffix, value) in [
            ("FUNCTION_INSTRUCTIONS", &mut limits.function_instructions),
            ("PREFIX_INSTRUCTIONS", &mut limits.prefix_instructions),
            ("CANDIDATES", &mut limits.candidates),
            ("MATCHING_CALLS", &mut limits.matching_calls),
            ("CALLER_INSTRUCTIONS", &mut limits.caller_instructions),
            ("DATAFLOW_STEPS", &mut limits.dataflow_steps),
        ] {
            let name = format!("ACTRAIL_RUSTLS_HANDSHAKE_{suffix}");
            match std::env::var(&name) {
                Ok(raw) => {
                    *value = raw
                        .parse::<usize>()
                        .ok()
                        .filter(|value| *value > 0)
                        .ok_or_else(|| {
                            DetectorConfigError::new(format!("{name} must be a positive integer"))
                        })?
                }
                Err(std::env::VarError::NotPresent) => {}
                Err(error) => return Err(DetectorConfigError::new(format!("{name}: {error}"))),
            }
        }
        if limits.prefix_instructions > limits.function_instructions {
            return Err(DetectorConfigError::new(
                "rustls handshake prefix budget exceeds function budget",
            ));
        }
        Ok(limits)
    }
}
