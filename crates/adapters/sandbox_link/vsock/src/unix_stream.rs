use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixListener;
use std::path::Path;

pub(super) fn bind(path: &Path, backlog: u32) -> io::Result<UnixListener> {
    validate_path(path)?;
    let fd = create_socket()?;
    let path_bytes = path.as_os_str().as_bytes();
    // SAFETY: zero is a valid base representation for sockaddr_un.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (target, source) in address.sun_path.iter_mut().zip(path_bytes.iter().copied()) {
        *target = source as libc::c_char;
    }
    let address_length = std::mem::offset_of!(libc::sockaddr_un, sun_path)
        .checked_add(path_bytes.len() + 1)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Unix path length overflow"))?;
    // SAFETY: address contains a NUL-terminated filesystem pathname and remains live for bind.
    if unsafe {
        libc::bind(
            fd.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            address_length as libc::socklen_t,
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is a bound AF_UNIX stream socket and backlog was validated by the caller.
    if unsafe { libc::listen(fd.as_raw_fd(), backlog as i32) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(UnixListener::from(fd))
}

pub(super) fn validate_path(path: &Path) -> io::Result<()> {
    let path_bytes = path.as_os_str().as_bytes();
    // SAFETY: zero is a valid base representation for reading sockaddr_un field capacity.
    let address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if path_bytes.is_empty()
        || path_bytes.contains(&0)
        || path_bytes.len() >= address.sun_path.len()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Unix socket path is empty, contains NUL, or exceeds sockaddr_un",
        ));
    }
    Ok(())
}

fn create_socket() -> io::Result<OwnedFd> {
    // SAFETY: socket is called with a supported address family/type and no pointers.
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: socket returned a new owned descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}
