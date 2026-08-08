//! Shared ownership of seccomp user-notify listener file descriptors.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Arc;

use config_core::daemon::SeccompNotifyConfig;
use control_contract::reply::ControlError;
use model_core::ids::TraceId;

use super::notify::{
    ListenerReadiness, SeccompRecv, continue_notification, deny_notification_errno,
    listener_readiness, notification_id_valid, recv_notification, validate_seccomp_notif_abi,
};

#[derive(Debug)]
pub(crate) struct SeccompNotifyService {
    enabled: bool,
    listeners: Vec<SeccompListener>,
}

impl SeccompNotifyService {
    pub(crate) fn new(config: &SeccompNotifyConfig) -> Self {
        Self {
            enabled: config.enabled,
            listeners: Vec::new(),
        }
    }

    pub(crate) fn register_listener(
        &mut self,
        trace_id: TraceId,
        listener_fd: Option<RawFd>,
    ) -> Result<(), ControlError> {
        if !self.enabled {
            return Err(ControlError::new(
                "seccomp_listener",
                "seccomp notify is not enabled",
            ));
        }
        let fd = listener_fd.ok_or_else(|| {
            ControlError::new(
                "seccomp_listener",
                "seccomp listener registration requires an SCM_RIGHTS listener fd",
            )
        })?;
        validate_seccomp_notif_abi()?;
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        self.listeners.push(SeccompListener {
            trace_id,
            fd: Arc::new(owned),
        });
        Ok(())
    }

    pub(crate) fn event_poll_fds(&self) -> Vec<RawFd> {
        self.listeners
            .iter()
            .map(|listener| listener.fd.as_raw_fd())
            .collect()
    }

    pub(crate) fn drain_notifications(
        &mut self,
        mut handler: impl FnMut(
            TraceId,
            &libc::seccomp_notif,
            &mut NotificationContinuation,
        ) -> Result<(), ControlError>,
    ) -> Result<(), ControlError> {
        if !self.enabled {
            return Ok(());
        }
        let mut index = 0;
        while index < self.listeners.len() {
            let listener = self.listeners[index].clone();
            let listener_fd = listener.fd.as_raw_fd();
            match listener_readiness(listener_fd)? {
                ListenerReadiness::Notification => {
                    if drain_listener(&listener, &mut handler)? {
                        self.listeners.remove(index);
                    } else {
                        index += 1;
                    }
                }
                ListenerReadiness::Closed => {
                    self.listeners.remove(index);
                }
                ListenerReadiness::Idle => {
                    index += 1;
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct SeccompListener {
    trace_id: TraceId,
    fd: Arc<OwnedFd>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContinuationState {
    Pending,
    Responded,
    Deferred,
}

#[derive(Debug)]
pub(crate) struct NotificationContinuation {
    listener: Arc<OwnedFd>,
    trace_id: TraceId,
    notification_id: u64,
    state: ContinuationState,
}

impl NotificationContinuation {
    fn new(listener: Arc<OwnedFd>, trace_id: TraceId, notification_id: u64) -> Self {
        Self {
            listener,
            trace_id,
            notification_id,
            state: ContinuationState::Pending,
        }
    }

    pub(crate) fn continue_now(&mut self) -> Result<(), ControlError> {
        if self.state != ContinuationState::Pending {
            return Ok(());
        }
        continue_notification(self.listener.as_raw_fd(), self.notification_id)?;
        self.state = ContinuationState::Responded;
        Ok(())
    }

    pub(crate) fn deny_errno(&mut self, errno: i32) -> Result<(), ControlError> {
        if self.state != ContinuationState::Pending {
            return Ok(());
        }
        deny_notification_errno(self.listener.as_raw_fd(), self.notification_id, errno)?;
        self.state = ContinuationState::Responded;
        Ok(())
    }

    pub(crate) fn defer(&mut self) -> Result<DeferredNotification, ControlError> {
        if self.state != ContinuationState::Pending {
            return Err(ControlError::new(
                "seccomp_notification",
                "notification has already been responded to or deferred",
            ));
        }
        self.state = ContinuationState::Deferred;
        Ok(DeferredNotification {
            listener: self.listener.clone(),
            notification_id: self.notification_id,
            trace_id: self.trace_id,
        })
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.state != ContinuationState::Pending
    }

    pub(crate) fn is_valid(&self) -> Result<bool, ControlError> {
        notification_id_valid(self.listener.as_raw_fd(), self.notification_id)
    }

    fn finish(&mut self) -> Result<(), ControlError> {
        if self.state == ContinuationState::Pending {
            self.continue_now()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DeferredNotification {
    listener: Arc<OwnedFd>,
    notification_id: u64,
    trace_id: TraceId,
}

impl DeferredNotification {
    pub(crate) fn trace_id(&self) -> TraceId {
        self.trace_id
    }

    pub(crate) fn notification_id(&self) -> u64 {
        self.notification_id
    }

    pub(crate) fn is_valid(&self) -> Result<bool, ControlError> {
        notification_id_valid(self.listener.as_raw_fd(), self.notification_id)
    }

    pub(crate) fn continue_now(&self) -> Result<(), ControlError> {
        continue_notification(self.listener.as_raw_fd(), self.notification_id)
    }

    pub(crate) fn deny_errno(&self, errno: i32) -> Result<(), ControlError> {
        deny_notification_errno(self.listener.as_raw_fd(), self.notification_id, errno)
    }
}

fn drain_listener(
    listener: &SeccompListener,
    handler: &mut impl FnMut(
        TraceId,
        &libc::seccomp_notif,
        &mut NotificationContinuation,
    ) -> Result<(), ControlError>,
) -> Result<bool, ControlError> {
    loop {
        match recv_notification(listener.fd.as_raw_fd())? {
            SeccompRecv::Ready(notification) => {
                let mut continuation = NotificationContinuation::new(
                    listener.fd.clone(),
                    listener.trace_id,
                    notification.id,
                );
                let handle_result = handler(listener.trace_id, &notification, &mut continuation);
                let continue_result = continuation.finish();
                handle_result?;
                continue_result?;
            }
            SeccompRecv::Drained => return Ok(false),
        }
        match listener_readiness(listener.fd.as_raw_fd())? {
            ListenerReadiness::Notification => {}
            ListenerReadiness::Closed => return Ok(true),
            ListenerReadiness::Idle => return Ok(false),
        }
    }
}
