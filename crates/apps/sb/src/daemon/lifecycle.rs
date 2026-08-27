use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::ptr;
use std::time::Duration;

pub(crate) enum DaemonEvent {
    StopRequested,
    ControlServerExited,
    DiagnosticsDue,
}

pub(crate) struct DaemonEventOwner {
    signal_fd: OwnedFd,
    previous_signal_mask: libc::sigset_t,
}

impl DaemonEventOwner {
    pub(crate) fn block_shutdown_signals() -> io::Result<Self> {
        let mut signals = MaybeUninit::<libc::sigset_t>::uninit();
        let mut previous = MaybeUninit::<libc::sigset_t>::uninit();
        unsafe {
            if libc::sigemptyset(signals.as_mut_ptr()) != 0 {
                return Err(io::Error::last_os_error());
            }
            let mut signals = signals.assume_init();
            if libc::sigaddset(&mut signals, libc::SIGINT) != 0
                || libc::sigaddset(&mut signals, libc::SIGTERM) != 0
            {
                return Err(io::Error::last_os_error());
            }
            let mask_result =
                libc::pthread_sigmask(libc::SIG_BLOCK, &signals, previous.as_mut_ptr());
            if mask_result != 0 {
                return Err(io::Error::from_raw_os_error(mask_result));
            }
            let previous_signal_mask = previous.assume_init();
            let raw_fd = libc::signalfd(-1, &signals, libc::SFD_CLOEXEC | libc::SFD_NONBLOCK);
            if raw_fd < 0 {
                let error = io::Error::last_os_error();
                let _ = libc::pthread_sigmask(
                    libc::SIG_SETMASK,
                    &previous_signal_mask,
                    ptr::null_mut(),
                );
                return Err(error);
            }
            Ok(Self {
                signal_fd: OwnedFd::from_raw_fd(raw_fd),
                previous_signal_mask,
            })
        }
    }

    pub(crate) fn wait(
        &self,
        control_health_fd: Option<RawFd>,
        diagnostics_wait: Option<Duration>,
    ) -> io::Result<DaemonEvent> {
        let mut poll_fds = [
            Self::poll_fd(self.signal_fd.as_raw_fd()),
            Self::poll_fd(control_health_fd.unwrap_or(-1)),
        ];
        let timeout = diagnostics_wait.map(Self::timespec);
        loop {
            let timeout_ptr = timeout
                .as_ref()
                .map_or(ptr::null(), |value| value as *const libc::timespec);
            let ready = unsafe {
                libc::ppoll(
                    poll_fds.as_mut_ptr(),
                    poll_fds.len() as libc::nfds_t,
                    timeout_ptr,
                    ptr::null(),
                )
            };
            if ready < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if poll_fds[0].revents & libc::POLLIN != 0 {
                self.consume_signal()?;
                return Ok(DaemonEvent::StopRequested);
            }
            if poll_fds[1].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR | libc::POLLNVAL)
                != 0
            {
                return Ok(DaemonEvent::ControlServerExited);
            }
            if ready == 0 {
                return Ok(DaemonEvent::DiagnosticsDue);
            }
        }
    }

    fn consume_signal(&self) -> io::Result<()> {
        let mut signal = MaybeUninit::<libc::signalfd_siginfo>::uninit();
        let expected = size_of::<libc::signalfd_siginfo>();
        let read = unsafe {
            libc::read(
                self.signal_fd.as_raw_fd(),
                signal.as_mut_ptr().cast(),
                expected,
            )
        };
        if read == expected as isize {
            Ok(())
        } else if read < 0 {
            Err(io::Error::last_os_error())
        } else {
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "actrail-sb received a truncated signal event",
            ))
        }
    }

    const fn poll_fd(fd: RawFd) -> libc::pollfd {
        libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        }
    }

    fn timespec(duration: Duration) -> libc::timespec {
        libc::timespec {
            tv_sec: duration.as_secs() as libc::time_t,
            tv_nsec: duration.subsec_nanos() as libc::c_long,
        }
    }
}

impl Drop for DaemonEventOwner {
    fn drop(&mut self) {
        unsafe {
            let _ = libc::pthread_sigmask(
                libc::SIG_SETMASK,
                &self.previous_signal_mask,
                ptr::null_mut(),
            );
        }
    }
}
