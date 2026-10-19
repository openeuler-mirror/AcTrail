use model_core::process::ProcessIdentity;

/// A verified observer coordinate supplied by the identity owner for one sample.
#[derive(Clone, Copy, Debug)]
pub struct VerifiedProcessTarget {
    pub(crate) identity: ProcessIdentity,
    pub(crate) observer_pid: u32,
    pub(crate) expected_start_time_ticks: u64,
}

impl VerifiedProcessTarget {
    pub fn identity(&self) -> ProcessIdentity {
        self.identity
    }
    pub fn observer_pid(&self) -> u32 {
        self.observer_pid
    }
    pub fn expected_start_time_ticks(&self) -> u64 {
        self.expected_start_time_ticks
    }

    pub fn new(
        identity: ProcessIdentity,
        observer_pid: u32,
        expected_start_time_ticks: u64,
    ) -> Self {
        Self {
            identity,
            observer_pid,
            expected_start_time_ticks,
        }
    }
}
