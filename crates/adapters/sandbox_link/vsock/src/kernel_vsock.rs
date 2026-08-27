use std::fs::File;
use std::io;
use std::mem::size_of;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::time::{Duration, Instant};

pub(super) fn connect(cid: u32, port: u32, timeout: Duration) -> io::Result<File> {
    let fd = create_socket()?;
    set_nonblocking(fd.as_raw_fd(), true)?;
    let address = address(cid, port);
    // SAFETY: fd is an owned AF_VSOCK socket and address points to a fully initialized
    // sockaddr_vm for the duration of this call.
    let result = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&address as *const libc::sockaddr_vm).cast(),
            size_of::<libc::sockaddr_vm>() as libc::socklen_t,
        )
    };
    if result < 0 {
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EISCONN) => {}
            Some(code)
                if code == libc::EINPROGRESS
                    || code == libc::EALREADY
                    || code == libc::EWOULDBLOCK
                    || code == libc::EINTR =>
            {
                wait_for_connect(fd.as_raw_fd(), timeout)?;
            }
            _ => return Err(error),
        }
    }
    set_nonblocking(fd.as_raw_fd(), false)?;
    Ok(File::from(fd))
}

fn wait_for_connect(fd: RawFd, timeout: Duration) -> io::Result<()> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "VSOCK timeout overflow"))?;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "VSOCK connect timed out",
            ));
        }
        let mut descriptor = libc::pollfd {
            fd,
            events: libc::POLLOUT,
            revents: 0,
        };
        // Round up so a positive sub-millisecond remainder cannot become an immediate busy poll.
        let partial_millisecond = if remaining.subsec_nanos() % 1_000_000 == 0 {
            0
        } else {
            1
        };
        let timeout_ms = remaining
            .as_millis()
            .saturating_add(partial_millisecond)
            .clamp(1, i32::MAX as u128) as libc::c_int;
        // SAFETY: descriptor is a valid one-element pollfd array for the duration of the call.
        let result = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
        if result > 0 {
            return connected_socket_result(fd);
        }
        if result == 0 {
            continue;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn connected_socket_result(fd: RawFd) -> io::Result<()> {
    let mut socket_error: libc::c_int = 0;
    let mut length = size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: socket_error and length are writable and have the SO_ERROR value layout.
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            (&mut socket_error as *mut libc::c_int).cast(),
            &mut length,
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    if socket_error != 0 {
        return Err(io::Error::from_raw_os_error(socket_error));
    }
    Ok(())
}

pub(super) fn bind(cid: u32, port: u32, backlog: u32) -> io::Result<File> {
    let fd = create_socket()?;
    let address = address(cid, port);
    // SAFETY: fd and sockaddr_vm are valid and owned for each syscall.
    let bind_result = unsafe {
        libc::bind(
            fd.as_raw_fd(),
            (&address as *const libc::sockaddr_vm).cast(),
            size_of::<libc::sockaddr_vm>() as libc::socklen_t,
        )
    };
    if bind_result < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is a bound stream socket.
    let listen_result = unsafe { libc::listen(fd.as_raw_fd(), backlog as i32) };
    if listen_result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(File::from(fd))
}

pub(super) fn accept(listener: RawFd) -> io::Result<(File, u32, u32)> {
    // SAFETY: zero is a valid initial representation for sockaddr_vm.
    let mut peer: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
    let mut length = size_of::<libc::sockaddr_vm>() as libc::socklen_t;
    // SAFETY: listener is held open by the caller; peer and length are writable.
    let accepted = unsafe {
        libc::accept4(
            listener,
            (&mut peer as *mut libc::sockaddr_vm).cast(),
            &mut length,
            libc::SOCK_CLOEXEC,
        )
    };
    if accepted < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: accept4 returned a new owned descriptor.
    let fd = unsafe { OwnedFd::from_raw_fd(accepted) };
    Ok((File::from(fd), peer.svm_cid, peer.svm_port))
}

pub(super) fn set_nonblocking(fd: RawFd, enabled: bool) -> io::Result<()> {
    // SAFETY: fcntl does not outlive fd and uses valid commands.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let next = if enabled {
        flags | libc::O_NONBLOCK
    } else {
        flags & !libc::O_NONBLOCK
    };
    // SAFETY: fd remains valid and next contains file status flags.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, next) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn set_timeouts(fd: RawFd, timeout: Duration) -> io::Result<()> {
    let subsecond_micros = if timeout.subsec_nanos() == 0 {
        0
    } else {
        timeout.subsec_micros().max(1)
    };
    let value = libc::timeval {
        tv_sec: timeout.as_secs().min(libc::time_t::MAX as u64) as libc::time_t,
        tv_usec: subsecond_micros as libc::suseconds_t,
    };
    for option in [libc::SO_RCVTIMEO, libc::SO_SNDTIMEO] {
        // SAFETY: fd is valid; value has the exact timeval layout and size.
        let result = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                option,
                (&value as *const libc::timeval).cast(),
                size_of::<libc::timeval>() as libc::socklen_t,
            )
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn create_socket() -> io::Result<OwnedFd> {
    // SAFETY: socket is called with a supported address family/type and no pointers.
    let raw = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: socket returned a new owned descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

fn address(cid: u32, port: u32) -> libc::sockaddr_vm {
    // SAFETY: zero is a valid base representation and every meaningful field is set below.
    let mut value: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
    value.svm_family = libc::AF_VSOCK as libc::sa_family_t;
    value.svm_cid = cid;
    value.svm_port = port;
    value
}

trait AsRawFdExt {
    fn as_raw_fd(&self) -> RawFd;
}

impl AsRawFdExt for OwnedFd {
    fn as_raw_fd(&self) -> RawFd {
        std::os::fd::AsRawFd::as_raw_fd(self)
    }
}
