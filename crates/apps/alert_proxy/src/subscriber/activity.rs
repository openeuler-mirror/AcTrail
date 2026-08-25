use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(super) struct SessionActivity {
    state: Mutex<ActivityState>,
}

struct ActivityState {
    last_peer_activity: Instant,
    last_ping: Instant,
    outstanding: Option<OutstandingPing>,
    next_nonce: u64,
}

struct OutstandingPing {
    nonce: u64,
    deadline: Instant,
}

pub(super) enum HeartbeatAction {
    None,
    Send { nonce: u64 },
    Close,
}

impl SessionActivity {
    pub(super) fn new() -> Self {
        let now = Instant::now();
        Self {
            state: Mutex::new(ActivityState {
                last_peer_activity: now,
                last_ping: now,
                outstanding: None,
                next_nonce: 1,
            }),
        }
    }

    pub(super) fn record_request(&self) -> Result<(), String> {
        self.state
            .lock()
            .map_err(|_| "subscriber activity lock is poisoned".to_string())?
            .last_peer_activity = Instant::now();
        Ok(())
    }

    pub(super) fn accept_pong(&self, nonce: u64) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "subscriber activity lock is poisoned".to_string())?;
        let Some(outstanding) = state.outstanding.as_ref() else {
            return Err("pong received without an outstanding ping".to_string());
        };
        if outstanding.nonce != nonce {
            return Err("pong nonce does not match the outstanding ping".to_string());
        }
        state.outstanding = None;
        state.last_peer_activity = Instant::now();
        Ok(())
    }

    pub(super) fn heartbeat_action(
        &self,
        heartbeat_interval: Duration,
        pong_timeout: Duration,
        peer_idle_timeout: Duration,
    ) -> HeartbeatAction {
        let Ok(mut state) = self.state.lock() else {
            return HeartbeatAction::Close;
        };
        let now = Instant::now();
        if now.duration_since(state.last_peer_activity) >= peer_idle_timeout {
            return HeartbeatAction::Close;
        }
        if state
            .outstanding
            .as_ref()
            .is_some_and(|ping| now >= ping.deadline)
        {
            return HeartbeatAction::Close;
        }
        if state.outstanding.is_none() && now.duration_since(state.last_ping) >= heartbeat_interval
        {
            let nonce = state.next_nonce;
            state.next_nonce = state.next_nonce.wrapping_add(1).max(1);
            state.last_ping = now;
            let Some(deadline) = now.checked_add(pong_timeout) else {
                return HeartbeatAction::Close;
            };
            state.outstanding = Some(OutstandingPing { nonce, deadline });
            return HeartbeatAction::Send { nonce };
        }
        HeartbeatAction::None
    }
}
