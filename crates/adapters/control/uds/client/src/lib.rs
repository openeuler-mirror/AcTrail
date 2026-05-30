//! Unix-domain-socket client adapter for control-plane consumers.

use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use control_contract::command::ControlCommand;
use control_contract::reply::{ControlError, ControlReply};

pub trait RoundTripTransport {
    fn send(&mut self, request: Vec<u8>) -> Result<Vec<u8>, String>;

    fn send_with_fds(&mut self, request: Vec<u8>, fds: &[RawFd]) -> Result<Vec<u8>, String> {
        if fds.is_empty() {
            self.send(request)
        } else {
            Err("transport does not support fd passing".to_string())
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UdsSocketTransport {
    socket_path: PathBuf,
}

impl UdsSocketTransport {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}

impl RoundTripTransport for UdsSocketTransport {
    fn send(&mut self, request: Vec<u8>) -> Result<Vec<u8>, String> {
        let mut stream =
            UnixStream::connect(&self.socket_path).map_err(|error| error.to_string())?;
        stream
            .write_all(&request)
            .and_then(|_| stream.shutdown(Shutdown::Write))
            .map_err(|error| error.to_string())?;

        let mut reply = Vec::new();
        stream
            .read_to_end(&mut reply)
            .map_err(|error| error.to_string())?;
        Ok(reply)
    }

    fn send_with_fds(&mut self, request: Vec<u8>, fds: &[RawFd]) -> Result<Vec<u8>, String> {
        if fds.is_empty() {
            return self.send(request);
        }
        let mut stream =
            UnixStream::connect(&self.socket_path).map_err(|error| error.to_string())?;
        send_request_with_fds(&stream, &request, fds)?;
        stream
            .shutdown(Shutdown::Write)
            .map_err(|error| error.to_string())?;

        let mut reply = Vec::new();
        stream
            .read_to_end(&mut reply)
            .map_err(|error| error.to_string())?;
        Ok(reply)
    }
}

pub struct UdsControlClient<T> {
    transport: T,
}

impl<T> UdsControlClient<T>
where
    T: RoundTripTransport,
{
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    pub fn send(&mut self, command: ControlCommand) -> Result<ControlReply, ControlError> {
        let fds = command_fds(&command);
        let request = uds_control_transport::encode_command(&command);
        let bytes = self
            .transport
            .send_with_fds(request, &fds)
            .map_err(|error| ControlError::new("transport", error))?;
        uds_control_transport::decode_reply(&bytes)
            .map_err(|error| ControlError::new(error.stage, error.message))?
    }
}

fn command_fds(command: &ControlCommand) -> Vec<RawFd> {
    match command {
        ControlCommand::RegisterSeccompListener(command) => {
            command.listener_fd.into_iter().collect()
        }
        _ => Vec::new(),
    }
}

fn send_request_with_fds(stream: &UnixStream, request: &[u8], fds: &[RawFd]) -> Result<(), String> {
    let mut iov = libc::iovec {
        iov_base: request.as_ptr().cast_mut().cast(),
        iov_len: request.len(),
    };
    let control_len =
        unsafe { libc::CMSG_SPACE(std::mem::size_of_val(fds) as libc::c_uint) } as usize;
    let mut control = vec![0_u8; control_len];
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut iov;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = control.len();

    unsafe {
        let header = libc::CMSG_FIRSTHDR(&message);
        if header.is_null() {
            return Err("build control message: missing header".to_string());
        }
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len = libc::CMSG_LEN(std::mem::size_of_val(fds) as libc::c_uint) as usize;
        std::ptr::copy_nonoverlapping(
            fds.as_ptr().cast::<u8>(),
            libc::CMSG_DATA(header),
            std::mem::size_of_val(fds),
        );
        let sent = libc::sendmsg(stream.as_raw_fd(), &message, 0);
        if sent < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if sent as usize != request.len() {
            return Err(format!(
                "short control request send: wrote {} of {} bytes",
                sent,
                request.len()
            ));
        }
    }
    Ok(())
}
