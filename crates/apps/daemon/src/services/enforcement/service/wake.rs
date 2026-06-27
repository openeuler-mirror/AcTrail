use std::io;
use std::os::fd::RawFd;

pub(super) struct WakeFd {
    fd: RawFd,
}

impl WakeFd {
    pub(super) fn new() -> Result<Self, String> {
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if fd < 0 {
            return Err(format!(
                "fanotify completion eventfd: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(Self { fd })
    }

    pub(super) fn fd(&self) -> RawFd {
        self.fd
    }

    pub(super) fn drain(&self) -> Result<(), String> {
        loop {
            let mut value = 0_u64;
            let read = unsafe {
                libc::read(
                    self.fd,
                    (&mut value as *mut u64).cast(),
                    std::mem::size_of::<u64>(),
                )
            };
            if read < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::WouldBlock {
                    return Ok(());
                }
                return Err(format!("fanotify completion eventfd read: {error}"));
            }
            if read == 0 {
                return Ok(());
            }
            if read as usize != std::mem::size_of::<u64>() {
                return Err(format!("fanotify completion eventfd short read: {read}"));
            }
        }
    }
}

impl Drop for WakeFd {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

pub(super) fn notify_wake_fd(fd: RawFd) -> Result<(), String> {
    let value = 1_u64;
    let written = unsafe {
        libc::write(
            fd,
            (&value as *const u64).cast(),
            std::mem::size_of::<u64>(),
        )
    };
    if written < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::WouldBlock {
            return Ok(());
        }
        return Err(format!("fanotify completion eventfd write: {error}"));
    }
    if written as usize != std::mem::size_of::<u64>() {
        return Err(format!(
            "fanotify completion eventfd short write: {written}"
        ));
    }
    Ok(())
}
