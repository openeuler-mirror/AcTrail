//! Application guidance for request-id argument failures.

use clap::error::{ContextKind, ContextValue};

pub(super) struct ArgumentDiagnostics;

impl ArgumentDiagnostics {
    pub(super) fn request_id(value: &str) -> Result<u64, String> {
        value.parse::<u64>().map_err(|error| {
            format!("expected an unsigned decimal integer in 0..=18446744073709551615: {error}")
        })
    }

    pub(super) fn exit(mut error: clap::Error, mut command: clap::Command) -> ! {
        command.build();
        let request_id_error = match error.get(ContextKind::InvalidArg) {
            Some(ContextValue::String(argument)) => command.get_arguments().any(|defined| {
                defined.get_id() == "request_id" && defined.to_string() == *argument
            }),
            _ => false,
        };
        if request_id_error {
            error.insert(ContextKind::Suggested, ContextValue::StyledStrs(vec![
                "--request-id requires an unsigned decimal integer in 0..=18446744073709551615; usage: actrailctl --request-id <ID> doctor".into(),
            ]));
        }
        error.exit()
    }
}
