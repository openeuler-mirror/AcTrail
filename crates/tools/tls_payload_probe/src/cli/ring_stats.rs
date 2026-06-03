//! Human-readable ring-buffer diagnostics.

use std::fmt::Write as FmtWrite;

use crate::ToolResult;
use crate::capture::RingStatsSnapshot;
use crate::cli::format::Style;
use crate::cli::output::Output;

const PERCENT_SCALE: f64 = 100.0;

pub(crate) struct RingStatsReporter {
    style: Style,
}

impl RingStatsReporter {
    pub(crate) fn new() -> Self {
        Self {
            style: Style::auto(),
        }
    }

    pub(crate) fn ring_stats(&self, stats: &RingStatsSnapshot) -> ToolResult<()> {
        let emitted = &stats.emitted;
        let lost = stats.lost;
        let mut output = String::new();
        let _ = writeln!(output, "{}:", self.style.ring_stats_label());
        let _ = writeln!(output, "  {}:", self.style.key("emitted"));
        let _ = writeln!(
            output,
            "    {} = {}",
            self.style.key("events"),
            emitted.events
        );
        let _ = writeln!(
            output,
            "    {} = {}",
            self.style.key("actual_bytes"),
            emitted.actual_bytes
        );
        let _ = writeln!(
            output,
            "    {} = {}",
            self.style.key("reserved_bytes"),
            emitted.reserved_bytes
        );
        let _ = writeln!(
            output,
            "    {} = {}",
            self.style.key("slack_bytes"),
            emitted.slack_bytes()
        );
        let _ = writeln!(
            output,
            "    {} = {}",
            self.style.key("utilization"),
            percent(emitted.actual_bytes, emitted.reserved_bytes)
        );
        let _ = writeln!(output, "    {}:", self.style.key("buckets"));
        for bucket in emitted.buckets() {
            let _ = writeln!(
                output,
                "      - class={} events={} actual_bytes={} reserved_bytes={} slack_bytes={}",
                bucket.class_size,
                bucket.events,
                bucket.actual_bytes,
                bucket.reserved_bytes,
                bucket.slack_bytes()
            );
        }
        let _ = writeln!(output, "  {}:", self.style.key("lost"));
        let _ = writeln!(
            output,
            "    {} = {}",
            self.style.key("reserve_fail_events"),
            lost.reserve_fail_events
        );
        let _ = writeln!(
            output,
            "    {} = {}",
            self.style.key("reserve_fail_actual_bytes"),
            lost.reserve_fail_actual_bytes
        );
        let _ = writeln!(
            output,
            "    {} = {}",
            self.style.key("reserve_fail_reserved_bytes"),
            lost.reserve_fail_reserved_bytes
        );
        let _ = writeln!(
            output,
            "    {} = {}",
            self.style.key("read_user_fail_events"),
            lost.read_user_fail_events
        );
        let _ = writeln!(
            output,
            "    {} = {}",
            self.style.key("read_user_fail_actual_bytes"),
            lost.read_user_fail_actual_bytes
        );
        let _ = writeln!(
            output,
            "    {} = {}",
            self.style.key("read_user_fail_reserved_bytes"),
            lost.read_user_fail_reserved_bytes
        );
        Output::stdout(&output)
    }
}

fn percent(numerator: u64, denominator: u64) -> String {
    if denominator == 0 {
        return "0.00%".to_string();
    }
    format!(
        "{:.2}%",
        (numerator as f64 * PERCENT_SCALE) / denominator as f64
    )
}
