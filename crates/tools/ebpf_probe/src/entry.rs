//! Top-level entry boundary for the eBPF probe tool.

use crate::args::{ProbeCommand, parse_args};
use crate::report::format_live_verification_report;

pub fn run_from_env() -> Result<(), String> {
    match parse_args(std::env::args().skip(1))? {
        ProbeCommand::VerifyLive(config) => {
            let report =
                crate::verify::run_live_verification(config).map_err(|error| error.to_string())?;
            println!("{}", format_live_verification_report(&report));
            Ok(())
        }
        ProbeCommand::Workload(config) => crate::workload::run_workload(config),
    }
}
