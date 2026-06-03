//! Human-readable CLI reporter.

use tls_probe_point_finder::ProbePointPlan;

use crate::ToolResult;
use crate::cli::output::Output;

pub(crate) fn target(plan: &ProbePointPlan) -> ToolResult<()> {
    Output::stdout(&format!(
        "target:\n  binary = {}\n  provider = {}\n  source = {}\n  hook_binary = {}\n",
        plan.target.binary.display(),
        plan.provider.as_str(),
        plan.source.as_str(),
        plan.binary.path.display()
    ))
}
