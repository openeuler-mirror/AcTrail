use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdleDetectionConfig {
    pub enabled: bool,
    pub threshold: Duration,
    pub poll_interval: Duration,
}

impl Default for IdleDetectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: Duration::from_secs(30),
            poll_interval: Duration::from_secs(2),
        }
    }
}

impl IdleDetectionConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        for duration in [self.threshold, self.poll_interval] {
            if duration.is_zero() {
                return Err("idle detection threshold and poll interval must be nonzero");
            }
            if Instant::now().checked_add(duration).is_none()
                || SystemTime::now().checked_add(duration).is_none()
            {
                return Err("idle detection duration exceeds clock range");
            }
        }
        Ok(())
    }
}
