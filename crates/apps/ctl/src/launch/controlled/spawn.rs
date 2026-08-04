//! Atomic pidfd child creation for controlled launches.

use std::os::fd::{FromRawFd, OwnedFd};

pub(super) enum PidfdSpawn {
    Child,
    Parent { child: libc::pid_t, pidfd: OwnedFd },
}

impl PidfdSpawn {
    pub(super) fn create() -> Result<Self, String> {
        let mut pidfd = -1;
        let mut operation = "clone3(CLONE_PIDFD)";
        let mut child = Self::clone3(&mut pidfd);
        if child < 0 {
            let clone3_error = std::io::Error::last_os_error();
            if clone3_error.raw_os_error() != Some(libc::ENOSYS) {
                return Err(format!("{operation} launch child: {clone3_error}"));
            }

            pidfd = -1;
            operation = "clone(CLONE_PIDFD)";
            child = Self::legacy_clone(&mut pidfd);
            if child < 0 {
                return Err(format!(
                    "{operation} launch child after clone3 ENOSYS: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }

        if child == 0 {
            return Ok(Self::Child);
        }
        if pidfd < 0 {
            super::terminate_child(child as libc::pid_t);
            return Err(format!("{operation} did not return a pidfd"));
        }

        Ok(Self::Parent {
            child: child as libc::pid_t,
            pidfd: unsafe { OwnedFd::from_raw_fd(pidfd) },
        })
    }

    fn clone3(pidfd: &mut libc::c_int) -> libc::c_long {
        let mut args: libc::clone_args = unsafe { std::mem::zeroed() };
        args.flags = libc::CLONE_PIDFD as u64;
        args.pidfd = (pidfd as *mut libc::c_int) as usize as u64;
        args.exit_signal = libc::SIGCHLD as u64;
        unsafe {
            libc::syscall(
                libc::SYS_clone3,
                &args as *const libc::clone_args,
                std::mem::size_of::<libc::clone_args>(),
            )
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn legacy_clone(pidfd: &mut libc::c_int) -> libc::c_long {
        unsafe {
            libc::syscall(
                libc::SYS_clone,
                (libc::CLONE_PIDFD | libc::SIGCHLD) as libc::c_ulong,
                std::ptr::null_mut::<libc::c_void>(),
                pidfd as *mut libc::c_int,
                std::ptr::null_mut::<libc::c_int>(),
                0 as libc::c_ulong,
            )
        }
    }

    #[cfg(target_arch = "aarch64")]
    fn legacy_clone(pidfd: &mut libc::c_int) -> libc::c_long {
        unsafe {
            libc::syscall(
                libc::SYS_clone,
                (libc::CLONE_PIDFD | libc::SIGCHLD) as libc::c_ulong,
                std::ptr::null_mut::<libc::c_void>(),
                pidfd as *mut libc::c_int,
                0 as libc::c_ulong,
                std::ptr::null_mut::<libc::c_int>(),
            )
        }
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("pidfd launch supports only x86_64 and aarch64 Linux targets");
