use std::time::{Instant, SystemTime};

#[derive(Clone, Copy, Debug)]
pub struct SamplingContext {
    pub sampled_at: Instant,
    pub observed_at: SystemTime,
}
