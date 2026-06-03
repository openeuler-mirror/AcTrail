//! CLI output sinks.

use std::fmt::Display;
use std::io::{self, IsTerminal, Write};

use crate::{ToolError, ToolResult};

const NO_COLOR_ENV: &str = "NO_COLOR";
const POLL_FD_COUNT: libc::nfds_t = 1;
const POLL_WAIT_FOREVER_MS: libc::c_int = -1;

pub(crate) struct Output;

impl Output {
    pub(crate) fn stdout(text: &str) -> ToolResult<()> {
        let mut stdout = io::stdout().lock();
        write_all(&mut stdout, libc::STDOUT_FILENO, text)
    }

    pub(crate) fn stderr(text: &str) -> ToolResult<()> {
        let mut stderr = io::stderr().lock();
        write_all(&mut stderr, libc::STDERR_FILENO, text)
    }

    pub(crate) fn stdout_supports_color() -> bool {
        io::stdout().is_terminal() && std::env::var_os(NO_COLOR_ENV).is_none()
    }
}

pub(crate) fn write_error(error: &dyn Display) -> ToolResult<()> {
    Output::stderr(&format!("error: {error}\n"))
}

fn write_all(output: &mut impl Write, fd: libc::c_int, text: &str) -> ToolResult<()> {
    let mut remaining = text.as_bytes();
    while !remaining.is_empty() {
        match output.write(remaining) {
            Ok(0) => return Err(ToolError::new("output write returned zero bytes")),
            Ok(written) => remaining = &remaining[written..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => wait_writable(fd)?,
            Err(error) => return Err(error.into()),
        }
    }
    flush(output, fd)
}

fn flush(output: &mut impl Write, fd: libc::c_int) -> ToolResult<()> {
    loop {
        match output.flush() {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => wait_writable(fd)?,
            Err(error) => return Err(error.into()),
        }
    }
}

fn wait_writable(fd: libc::c_int) -> ToolResult<()> {
    let mut pollfd = libc::pollfd {
        fd,
        events: libc::POLLOUT,
        revents: 0,
    };
    loop {
        let result = unsafe {
            libc::poll(
                std::ptr::addr_of_mut!(pollfd),
                POLL_FD_COUNT,
                POLL_WAIT_FOREVER_MS,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(ToolError::new(format!("poll output fd={fd}: {error}")));
        }
        if result == 0 {
            continue;
        }
        if pollfd.revents & libc::POLLOUT != 0 {
            return Ok(());
        }
        if pollfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(ToolError::new(format!(
                "output fd={fd} is not writable: revents=0x{:x}",
                pollfd.revents
            )));
        }
    }
}
