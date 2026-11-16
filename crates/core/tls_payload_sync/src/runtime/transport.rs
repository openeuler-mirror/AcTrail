use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{EventQueue, FdRegistration};
use crate::DEFAULT_EVENT_BATCH_BYTES;

pub(crate) struct Endpoint(pub(crate) PathBuf);

impl Endpoint {
    pub(crate) fn connect(&self, deadline: Instant) -> io::Result<UnixStream> {
        let path = &self.0;
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        let path = path.as_os_str().as_bytes();
        if path.is_empty() || path.contains(&0) || path.len() >= address.sun_path.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid sync event socket path",
            ));
        }
        address.sun_family = libc::AF_UNIX as _;
        for (target, byte) in address.sun_path.iter_mut().zip(path) {
            *target = *byte as _;
        }
        let fd = unsafe {
            libc::socket(
                libc::AF_UNIX,
                libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                0,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let stream = unsafe { UnixStream::from_raw_fd(fd) };
        let length =
            (std::mem::offset_of!(libc::sockaddr_un, sun_path) + path.len() + 1) as libc::socklen_t;
        loop {
            DeadlineStream::remaining(deadline)?;
            let result =
                unsafe { libc::connect(fd, &address as *const _ as *const libc::sockaddr, length) };
            if result == 0 {
                break;
            }
            let error = io::Error::last_os_error();
            // AF_UNIX EAGAIN means the listener backlog is full, not an in-progress connection.
            if error.raw_os_error() == Some(libc::EAGAIN) {
                std::thread::sleep(
                    DeadlineStream::remaining(deadline)?.min(Duration::from_millis(1)),
                );
                continue;
            }
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if error.raw_os_error() != Some(libc::EINPROGRESS) {
                return Err(error);
            }
            DeadlineStream::wait_writable(&stream, deadline)?;
            if let Some(error) = stream.take_error()? {
                return Err(error);
            }
            stream.peer_addr()?;
            break;
        }
        Ok(stream)
    }

    pub(super) fn diagnostic(message: &str) {
        let mut metadata: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(libc::STDERR_FILENO, &mut metadata) } < 0 {
            return;
        }
        if metadata.st_mode & libc::S_IFMT == libc::S_IFREG {
            // Preserve the shared file offset; reopening a redirected file would let later
            // application writes overwrite this diagnostic. Only the disposable worker writes.
            let _ =
                unsafe { libc::write(libc::STDERR_FILENO, message.as_ptr().cast(), message.len()) };
            return;
        }
        if metadata.st_mode & libc::S_IFMT == libc::S_IFSOCK {
            let _ = unsafe {
                libc::send(
                    libc::STDERR_FILENO,
                    message.as_ptr().cast(),
                    message.len(),
                    libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                )
            };
            return;
        }
        // Reopen with a separate file description so O_NONBLOCK does not change application stderr.
        // A pipe reader can disappear between open and write. Keep SIGPIPE local to this
        // disposable worker rather than terminating an application with the default handler.
        let mut blocked: libc::sigset_t = unsafe { std::mem::zeroed() };
        unsafe {
            libc::sigemptyset(&mut blocked);
            libc::sigaddset(&mut blocked, libc::SIGPIPE);
            if libc::pthread_sigmask(libc::SIG_BLOCK, &blocked, std::ptr::null_mut()) != 0 {
                return;
            }
        }
        let fd = unsafe {
            libc::open(
                c"/proc/self/fd/2".as_ptr(),
                libc::O_WRONLY
                    | libc::O_APPEND
                    | libc::O_NONBLOCK
                    | libc::O_CLOEXEC
                    | libc::O_NOCTTY,
            )
        };
        if fd >= 0 {
            let fd = unsafe { OwnedFd::from_raw_fd(fd) };
            let _ = unsafe { libc::write(fd.as_raw_fd(), message.as_ptr().cast(), message.len()) };
        }
    }
}

pub(crate) struct DeadlineStream {
    _registration: Option<FdRegistration>,
    stream: UnixStream,
    deadline: Instant,
    progress: Option<(Duration, Arc<EventQueue>)>,
    write_max_bytes: usize,
}

impl DeadlineStream {
    pub(crate) fn new(stream: UnixStream) -> Self {
        Self {
            _registration: None,
            stream,
            deadline: Instant::now(),
            progress: None,
            write_max_bytes: DEFAULT_EVENT_BATCH_BYTES as usize,
        }
    }

    pub(super) fn for_events(
        stream: UnixStream,
        timeout: Duration,
        queue: Arc<EventQueue>,
        registration: FdRegistration,
        write_max_bytes: usize,
    ) -> Self {
        Self {
            _registration: Some(registration),
            stream,
            deadline: Instant::now(),
            progress: Some((timeout, queue)),
            write_max_bytes,
        }
    }

    pub(crate) fn set_deadline(&mut self, deadline: Instant) {
        self.deadline = deadline;
    }

    fn remaining(deadline: Instant) -> io::Result<std::time::Duration> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "sync event transport deadline exceeded",
            ));
        }
        Ok(remaining)
    }

    fn wait_writable(stream: &UnixStream, deadline: Instant) -> io::Result<()> {
        Self::wait_ready(stream, deadline, libc::POLLOUT)
    }

    fn wait_ready(stream: &UnixStream, deadline: Instant, events: libc::c_short) -> io::Result<()> {
        loop {
            let remaining = Self::remaining(deadline)?;
            let timeout = remaining
                .as_millis()
                .saturating_add(1)
                .min(i32::MAX as u128) as i32;
            let mut descriptor = libc::pollfd {
                fd: stream.as_raw_fd(),
                events,
                revents: 0,
            };
            let result = unsafe { libc::poll(&mut descriptor, 1, timeout) };
            if result > 0 {
                Self::remaining(deadline)?;
                return Ok(());
            }
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
    }
}

impl Drop for DeadlineStream {
    fn drop(&mut self) {
        if self.progress.is_some() {
            // Wake the release reader and let the daemon see EOF when the writer stops.
            let _ = self.stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

impl Read for DeadlineStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            Self::remaining(self.deadline)?;
            match self.stream.read(bytes) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    Self::wait_ready(&self.stream, self.deadline, libc::POLLIN)?;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => return result,
            }
        }
    }
}

impl Write for DeadlineStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            if let Some((_, queue)) = &self.progress {
                self.deadline = queue.deadline(self.deadline)?;
            }
            Self::remaining(self.deadline)?;
            match self
                .stream
                .write(&bytes[..bytes.len().min(self.write_max_bytes)])
            {
                Ok(written) => {
                    if written != 0 {
                        if let Some((timeout, queue)) = &self.progress {
                            self.deadline = queue.deadline(Instant::now() + *timeout)?;
                        }
                    }
                    return Ok(written);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    Self::wait_writable(&self.stream, self.deadline)?;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => return result,
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
