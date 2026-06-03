//! Linux seccomp user-notify operations.

use control_contract::reply::ControlError;

#[derive(Debug)]
pub(crate) enum ListenerReadiness {
    Notification,
    Closed,
    Idle,
}

#[derive(Debug)]
pub(crate) enum SeccompRecv {
    Ready(libc::seccomp_notif),
    Drained,
}

pub(crate) fn listener_readiness(fd: libc::c_int) -> Result<ListenerReadiness, ControlError> {
    let mut pollfd = libc::pollfd {
        fd,
        events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut pollfd, 1, 0) };
    if result < 0 {
        return Err(ControlError::new(
            "seccomp_listener_poll",
            std::io::Error::last_os_error().to_string(),
        ));
    }
    if result == 0 {
        return Ok(ListenerReadiness::Idle);
    }
    if pollfd.revents & libc::POLLIN != 0 {
        return Ok(ListenerReadiness::Notification);
    }
    if pollfd.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
        return Ok(ListenerReadiness::Closed);
    }
    Ok(ListenerReadiness::Idle)
}

pub(crate) fn recv_notification(fd: libc::c_int) -> Result<SeccompRecv, ControlError> {
    let mut notification: libc::seccomp_notif = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::ioctl(fd, seccomp_ioctl_notif_recv(), &mut notification) };
    if result == 0 {
        return Ok(SeccompRecv::Ready(notification));
    }
    let error = std::io::Error::last_os_error();
    if recv_error_is_recoverable(&error) {
        return Ok(SeccompRecv::Drained);
    }
    Err(ControlError::new("seccomp_notif_recv", error.to_string()))
}

pub(crate) fn continue_notification(fd: libc::c_int, id: u64) -> Result<(), ControlError> {
    let mut response = libc::seccomp_notif_resp {
        id,
        val: 0,
        error: 0,
        flags: libc::SECCOMP_USER_NOTIF_FLAG_CONTINUE as u32,
    };
    let result = unsafe { libc::ioctl(fd, seccomp_ioctl_notif_send(), &mut response) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if seccomp_notification_is_stale(&error) {
        return Ok(());
    }
    Err(ControlError::new("seccomp_notif_send", error.to_string()))
}

pub(crate) fn validate_seccomp_notif_abi() -> Result<(), ControlError> {
    let mut sizes: libc::seccomp_notif_sizes = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::syscall(
            libc::SYS_seccomp,
            libc::SECCOMP_GET_NOTIF_SIZES,
            0,
            &mut sizes,
        )
    };
    if result != 0 {
        return Err(ControlError::new(
            "seccomp_notif_sizes",
            std::io::Error::last_os_error().to_string(),
        ));
    }
    validate_seccomp_notif_size::<libc::seccomp_notif>(sizes.seccomp_notif, "seccomp_notif")?;
    validate_seccomp_notif_size::<libc::seccomp_notif_resp>(
        sizes.seccomp_notif_resp,
        "seccomp_notif_resp",
    )?;
    validate_seccomp_notif_size::<libc::seccomp_data>(sizes.seccomp_data, "seccomp_data")
}

fn validate_seccomp_notif_size<T>(actual: u16, label: &'static str) -> Result<(), ControlError> {
    let expected = std::mem::size_of::<T>();
    let actual = usize::from(actual);
    if actual == expected {
        Ok(())
    } else {
        Err(ControlError::new(
            "seccomp_notif_sizes",
            format!("{label} ABI size mismatch: kernel={actual} libc={expected}"),
        ))
    }
}

const IOC_NRBITS: u64 = 8;
const IOC_TYPEBITS: u64 = 8;
const IOC_SIZEBITS: u64 = 14;
const IOC_NRSHIFT: u64 = 0;
const IOC_TYPESHIFT: u64 = IOC_NRSHIFT + IOC_NRBITS;
const IOC_SIZESHIFT: u64 = IOC_TYPESHIFT + IOC_TYPEBITS;
const IOC_DIRSHIFT: u64 = IOC_SIZESHIFT + IOC_SIZEBITS;
const IOC_WRITE: u64 = 1;
const IOC_READ: u64 = 2;
const SECCOMP_IOCTL_MAGIC: u64 = b'!' as u64;

const fn ioc(dir: u64, ty: u64, nr: u64, size: u64) -> libc::c_ulong {
    ((dir << IOC_DIRSHIFT) | (ty << IOC_TYPESHIFT) | (nr << IOC_NRSHIFT) | (size << IOC_SIZESHIFT))
        as libc::c_ulong
}

const fn seccomp_ioctl_notif_recv() -> libc::c_ulong {
    ioc(
        IOC_READ | IOC_WRITE,
        SECCOMP_IOCTL_MAGIC,
        0,
        std::mem::size_of::<libc::seccomp_notif>() as u64,
    )
}

const fn seccomp_ioctl_notif_send() -> libc::c_ulong {
    ioc(
        IOC_READ | IOC_WRITE,
        SECCOMP_IOCTL_MAGIC,
        1,
        std::mem::size_of::<libc::seccomp_notif_resp>() as u64,
    )
}

fn seccomp_notification_is_stale(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(errno) if errno == libc::ENOENT || errno == libc::ESRCH)
}

/// A `SECCOMP_IOCTL_NOTIF_RECV` failure that should be ignored rather than crash the daemon:
/// `EAGAIN` (no notification ready), `EINTR` (interrupted ioctl), or a stale id whose target
/// already exited (`ENOENT`/`ESRCH`, per `seccomp(2)`).
fn recv_error_is_recoverable(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock
        || error.raw_os_error() == Some(libc::EINTR)
        || seccomp_notification_is_stale(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_seccomp_notification_errors_are_nonfatal() {
        assert!(seccomp_notification_is_stale(
            &std::io::Error::from_raw_os_error(libc::ENOENT),
        ));
        assert!(seccomp_notification_is_stale(
            &std::io::Error::from_raw_os_error(libc::ESRCH),
        ));
        assert!(!seccomp_notification_is_stale(
            &std::io::Error::from_raw_os_error(libc::EBADF),
        ));
    }

    #[test]
    fn recv_errors_drained_for_recoverable_errnos() {
        for errno in [libc::EAGAIN, libc::EINTR, libc::ENOENT, libc::ESRCH] {
            assert!(
                recv_error_is_recoverable(&std::io::Error::from_raw_os_error(errno)),
                "errno {errno} should be recoverable",
            );
        }
        assert!(!recv_error_is_recoverable(
            &std::io::Error::from_raw_os_error(libc::EBADF),
        ));
    }
}
