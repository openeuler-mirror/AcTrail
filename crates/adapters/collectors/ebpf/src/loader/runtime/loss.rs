//! One bounded diagnostic window shared by the collector's loss producers.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::time::{Duration, Instant};

#[derive(Default)]
struct LossCount {
    count: u64,
    example: Option<String>,
}

pub(super) struct LossDiagnostics {
    // Keys are static failure categories, never paths, PIDs, or error messages.
    // Producers retain cumulative baselines; this owns only undelivered deltas.
    pending: BTreeMap<&'static str, LossCount>,
    interval: Duration,
    window_started: Option<Instant>,
    new_loss: bool,
}

impl LossDiagnostics {
    pub(super) fn new(interval_ms: u32) -> Self {
        Self {
            pending: BTreeMap::new(),
            interval: Duration::from_millis(u64::from(interval_ms)),
            window_started: None,
            new_loss: false,
        }
    }

    pub(super) fn record_count(&mut self, category: &'static str, count: u64) {
        if count == 0 {
            return;
        }
        let entry = self.pending.entry(category).or_default();
        entry.count = entry.count.saturating_add(count);
        self.window_started.get_or_insert_with(Instant::now);
        self.new_loss = true;
    }

    pub(super) fn record_error(
        &mut self,
        category: &'static str,
        example: impl FnOnce() -> String,
    ) {
        self.record_detail(category, 1, example);
    }

    pub(super) fn record_detail(
        &mut self,
        category: &'static str,
        count: u64,
        example: impl FnOnce() -> String,
    ) {
        self.record_count(category, count);
        if let Some(entry) = self.pending.get_mut(category)
            && entry.example.is_none()
        {
            entry.example = Some(example());
        }
    }

    pub(super) fn poll_timeout(&self) -> Option<Duration> {
        self.window_started
            .map(|started| self.interval.saturating_sub(started.elapsed()))
    }

    pub(super) fn take_summary(&mut self, force: bool) -> (bool, Option<String>) {
        let new_loss = std::mem::take(&mut self.new_loss);
        let Some(remaining) = self.poll_timeout() else {
            return (new_loss, None);
        };
        if !force && !remaining.is_zero() {
            return (new_loss, None);
        }
        let mut message = String::from("collector data loss since previous report: ");
        for (index, (category, entry)) in self.pending.iter().enumerate() {
            if index != 0 {
                message.push_str("; ");
            }
            let _ = write!(message, "{category}={}", entry.count);
            if let Some(example) = &entry.example {
                let _ = write!(message, " (first error: {example})");
            }
        }
        self.pending.clear();
        self.window_started = None;
        (new_loss, Some(message))
    }
}
