pub const DEFAULT_POST_TRACE_MAX_IN_FLIGHT_TASKS: u32 = 64;
pub const DEFAULT_POST_TRACE_BROKER_QUEUE_CAPACITY: u32 = 256;
pub const DEFAULT_POST_TRACE_REQUESTS_PER_CYCLE: u32 = 128;
pub const DEFAULT_POST_TRACE_BROKER_REPLY_TIMEOUT_MS: u64 = 5_000;
pub const DEFAULT_POST_TRACE_ADMISSION_TIMEOUT_MS: u64 = 30_000;
pub const DEFAULT_POST_TRACE_EXECUTION_TIMEOUT_MS: u64 = 60_000;
pub const DEFAULT_POST_TRACE_SHUTDOWN_DRAIN_TIMEOUT_MS: u64 = 30_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PostTraceRuntimeConfig {
    pub max_in_flight_tasks: u32,
    pub broker_queue_capacity: u32,
    pub requests_per_cycle: u32,
    pub broker_reply_timeout_ms: u64,
    pub admission_timeout_ms: u64,
    pub execution_timeout_ms: u64,
    pub shutdown_drain_timeout_ms: u64,
}

impl Default for PostTraceRuntimeConfig {
    fn default() -> Self {
        Self {
            max_in_flight_tasks: DEFAULT_POST_TRACE_MAX_IN_FLIGHT_TASKS,
            broker_queue_capacity: DEFAULT_POST_TRACE_BROKER_QUEUE_CAPACITY,
            requests_per_cycle: DEFAULT_POST_TRACE_REQUESTS_PER_CYCLE,
            broker_reply_timeout_ms: DEFAULT_POST_TRACE_BROKER_REPLY_TIMEOUT_MS,
            admission_timeout_ms: DEFAULT_POST_TRACE_ADMISSION_TIMEOUT_MS,
            execution_timeout_ms: DEFAULT_POST_TRACE_EXECUTION_TIMEOUT_MS,
            shutdown_drain_timeout_ms: DEFAULT_POST_TRACE_SHUTDOWN_DRAIN_TIMEOUT_MS,
        }
    }
}
