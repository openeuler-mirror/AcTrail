use std::io;
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use super::DaemonRunError;

pub(super) struct ControlLoopTiming {
    retry_delay: Duration,
    retry_at: Option<Instant>,
    background_at: Option<Instant>,
}

impl ControlLoopTiming {
    pub(super) fn new(retry_delay_ms: u32) -> Self {
        Self {
            retry_delay: Duration::from_millis(u64::from(retry_delay_ms)),
            retry_at: None,
            background_at: None,
        }
    }

    pub(super) fn poll_options(
        &mut self,
        listener_fd: RawFd,
        background_timeout: Option<Duration>,
    ) -> Result<(RawFd, Option<Duration>), DaemonRunError> {
        if background_timeout.is_none() && self.retry_at.is_none() {
            self.background_at = None;
            return Ok((listener_fd, None));
        }
        let now = Instant::now();
        self.background_at = match background_timeout {
            Some(timeout) => {
                let deadline = now.checked_add(timeout).ok_or_else(|| {
                    DaemonRunError::new("poll_timeout", "background poll deadline overflow")
                })?;
                // Readiness or accept retries must not postpone fixed-interval background work.
                Some(
                    self.background_at
                        .map_or(deadline, |current| current.min(deadline)),
                )
            }
            None => None,
        };
        let background_remaining = self
            .background_at
            .map(|deadline| deadline.saturating_duration_since(now));
        let retry_remaining = self
            .retry_at
            .and_then(|deadline| deadline.checked_duration_since(now));
        let timeout = match (background_remaining, retry_remaining) {
            (Some(background), Some(retry)) => Some(background.min(retry)),
            (background, retry) => background.or(retry),
        };
        // poll ignores a negative descriptor; existing clients and event sources stay active.
        Ok((
            if retry_remaining.is_some() {
                -1
            } else {
                listener_fd
            },
            timeout,
        ))
    }

    pub(super) fn record_accept_error(&mut self, error: io::Error) -> Result<(), DaemonRunError> {
        if matches!(
            error.raw_os_error(),
            Some(libc::EBADF | libc::ENOTSOCK | libc::EINVAL)
        ) {
            return Err(DaemonRunError::from_io("accept", error));
        }
        if self.retry_at.is_none() {
            tracing::warn!(
                %error,
                retry_delay_ms = self.retry_delay.as_millis(),
                "control listener accept failed; retrying while existing traces continue"
            );
        }
        self.retry_at = Some(
            Instant::now()
                .checked_add(self.retry_delay)
                .ok_or_else(|| {
                    DaemonRunError::new("poll_timeout", "control accept retry deadline overflow")
                })?,
        );
        Ok(())
    }

    pub(super) fn record_accept_success(&mut self) {
        if self.retry_at.take().is_some() {
            tracing::info!("control listener recovered");
        }
    }

    pub(super) fn record_background_progress(&mut self) {
        self.background_at = None;
    }

    pub(super) fn background_due(&self) -> bool {
        self.background_at
            .is_some_and(|deadline| Instant::now() >= deadline)
    }
}
