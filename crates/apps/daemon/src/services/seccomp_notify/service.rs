//! Shared ownership of seccomp user-notify listener file descriptors.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use config_core::daemon::SeccompNotifyConfig;
use control_contract::reply::ControlError;

use super::notify::{
    ListenerReadiness, SeccompRecv, continue_notification, listener_readiness, recv_notification,
    validate_seccomp_notif_abi,
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
        self.listeners.push(SeccompListener { fd: owned });
        Ok(())
    }

    pub(crate) fn event_poll_fds(&self) -> Vec<RawFd> {
        self.listeners
            .iter()
            .map(|listener| listener.fd.as_raw_fd())
            .collect()
    }

    pub(crate) fn has_listeners(&self) -> bool {
        !self.listeners.is_empty()
    }

    pub(crate) fn drain_notifications(
        &mut self,
        mut handler: impl FnMut(
            &libc::seccomp_notif,
            &mut NotificationContinuation,
        ) -> Result<(), ControlError>,
    ) -> Result<(), ControlError> {
        if !self.enabled {
            return Ok(());
        }
        let mut index = 0;
        while index < self.listeners.len() {
            let listener_fd = self.listeners[index].fd.as_raw_fd();
            match listener_readiness(listener_fd)? {
                ListenerReadiness::Notification => {
                    if drain_listener(listener_fd, &mut handler)? {
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

#[derive(Debug)]
struct SeccompListener {
    fd: OwnedFd,
}

#[derive(Debug)]
pub(crate) struct NotificationContinuation {
    listener_fd: RawFd,
    notification_id: u64,
    continued: bool,
}

impl NotificationContinuation {
    fn new(listener_fd: RawFd, notification_id: u64) -> Self {
        Self {
            listener_fd,
            notification_id,
            continued: false,
        }
    }

    pub(crate) fn continue_now(&mut self) -> Result<(), ControlError> {
        if self.continued {
            return Ok(());
        }
        continue_notification(self.listener_fd, self.notification_id)?;
        self.continued = true;
        Ok(())
    }

    fn finish(&mut self) -> Result<(), ControlError> {
        self.continue_now()
    }
}

fn drain_listener(
    listener_fd: RawFd,
    handler: &mut impl FnMut(
        &libc::seccomp_notif,
        &mut NotificationContinuation,
    ) -> Result<(), ControlError>,
) -> Result<bool, ControlError> {
    loop {
        match recv_notification(listener_fd)? {
            SeccompRecv::Ready(notification) => {
                let mut continuation = NotificationContinuation::new(listener_fd, notification.id);
                let handle_result = handler(&notification, &mut continuation);
                let continue_result = continuation.finish();
                handle_result?;
                continue_result?;
            }
            SeccompRecv::Drained => return Ok(false),
        }
        match listener_readiness(listener_fd)? {
            ListenerReadiness::Notification => {}
            ListenerReadiness::Closed => return Ok(true),
            ListenerReadiness::Idle => return Ok(false),
        }
    }
}
