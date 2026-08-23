use std::collections::{BTreeMap, BTreeSet};

use sandbox_observation::{CpuSnapshot, GuestBootId};
use sandbox_plugin_delivery::SandboxSource;

pub(super) struct SourceStateTable {
    capacity: usize,
    clock: u64,
    states: BTreeMap<SandboxSource, SourceState>,
    recency: BTreeSet<(u64, SandboxSource)>,
}

struct SourceState {
    last_seen: u64,
    guest_boot_id: Option<GuestBootId>,
    last_cpu: Option<CpuSnapshot>,
    cpu_risk_active: bool,
    last_oom_kill_count: Option<u64>,
    memory_risk_active: bool,
}

impl SourceStateTable {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            clock: 0,
            states: BTreeMap::new(),
            recency: BTreeSet::new(),
        }
    }

    pub(super) fn begin_batch(&mut self, source: SandboxSource) -> Result<(), StateError> {
        let next_clock = self
            .clock
            .checked_add(1)
            .ok_or(StateError::ClockExhausted)?;
        if let Some(state) = self.states.get(&source) {
            self.recency.remove(&(state.last_seen, source));
        } else if self.states.len() >= self.capacity {
            let oldest = self
                .recency
                .iter()
                .next()
                .copied()
                .ok_or(StateError::InvariantViolation)?;
            self.recency.remove(&oldest);
            if self.states.remove(&oldest.1).is_none() {
                return Err(StateError::InvariantViolation);
            }
        }
        self.clock = next_clock;
        let state = self.states.entry(source).or_insert(SourceState {
            last_seen: next_clock,
            guest_boot_id: None,
            last_cpu: None,
            cpu_risk_active: false,
            last_oom_kill_count: None,
            memory_risk_active: false,
        });
        state.last_seen = next_clock;
        self.recency.insert((next_clock, source));
        Ok(())
    }

    pub(super) fn update_cpu_sample(
        &mut self,
        source: SandboxSource,
        guest_boot_id: GuestBootId,
        current: CpuSnapshot,
    ) -> Result<Option<u16>, StateError> {
        let state = self
            .states
            .get_mut(&source)
            .ok_or(StateError::InvariantViolation)?;
        if state.guest_boot_id != Some(guest_boot_id) {
            state.reset_for_boot(guest_boot_id, current);
            return Ok(None);
        }
        let previous = state.last_cpu.replace(current);
        let Some(previous) = previous else {
            return Ok(None);
        };
        if current.total_ticks < previous.total_ticks || current.idle_ticks < previous.idle_ticks {
            return Ok(None);
        }
        let total_delta = current.total_ticks - previous.total_ticks;
        let idle_delta = current.idle_ticks - previous.idle_ticks;
        if total_delta == 0 || idle_delta > total_delta {
            return Ok(None);
        }
        let busy_delta = total_delta - idle_delta;
        let usage = u128::from(busy_delta)
            .saturating_mul(10_000)
            .checked_div(u128::from(total_delta))
            .unwrap_or_default();
        Ok(Some(u16::try_from(usage).unwrap_or(10_000)))
    }

    pub(super) fn update_cpu_risk(
        &mut self,
        source: SandboxSource,
        is_risk: bool,
    ) -> Result<bool, StateError> {
        let state = self
            .states
            .get_mut(&source)
            .ok_or(StateError::InvariantViolation)?;
        let entered = is_risk && !state.cpu_risk_active;
        state.cpu_risk_active = is_risk;
        Ok(entered)
    }

    pub(super) fn update_oom_kill_count(
        &mut self,
        source: SandboxSource,
        current_count: u64,
    ) -> Result<Option<OomIncrement>, StateError> {
        let state = self
            .states
            .get_mut(&source)
            .ok_or(StateError::InvariantViolation)?;
        let previous_count = state.last_oom_kill_count.replace(current_count);
        Ok(previous_count.and_then(|previous_count| {
            current_count
                .checked_sub(previous_count)
                .filter(|delta| *delta > 0)
                .map(|delta| OomIncrement {
                    previous_count,
                    current_count,
                    delta,
                })
        }))
    }

    pub(super) fn update_memory_risk(
        &mut self,
        source: SandboxSource,
        is_risk: bool,
    ) -> Result<bool, StateError> {
        let state = self
            .states
            .get_mut(&source)
            .ok_or(StateError::InvariantViolation)?;
        let entered = is_risk && !state.memory_risk_active;
        state.memory_risk_active = is_risk;
        Ok(entered)
    }
}

impl SourceState {
    fn reset_for_boot(&mut self, guest_boot_id: GuestBootId, cpu: CpuSnapshot) {
        self.guest_boot_id = Some(guest_boot_id);
        self.last_cpu = Some(cpu);
        self.cpu_risk_active = false;
        self.last_oom_kill_count = None;
        self.memory_risk_active = false;
    }
}

pub(super) struct OomIncrement {
    pub(super) previous_count: u64,
    pub(super) current_count: u64,
    pub(super) delta: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StateError {
    ClockExhausted,
    InvariantViolation,
}
