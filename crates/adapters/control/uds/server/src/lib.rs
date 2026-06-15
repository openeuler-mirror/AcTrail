//! Unix-domain-socket server adapter for daemon-side control handling.

use std::io::Write;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;

use control_contract::command::ControlCommand;
use control_contract::reply::{ControlError, ControlReply};

pub trait ControlService {
    fn handle(&mut self, command: ControlCommand) -> Result<ControlReply, ControlError>;
}

pub struct UdsControlServer<S> {
    service: S,
}

pub struct UdsControlConnection {
    stream: UnixStream,
    reply: Option<Vec<u8>>,
    written: usize,
}

impl<S> UdsControlServer<S>
where
    S: ControlService,
{
    pub fn new(service: S) -> Self {
        Self { service }
    }

    pub fn handle_bytes(&mut self, request: &[u8]) -> Vec<u8> {
        self.handle_bytes_with_fds(request, Vec::new())
    }

    pub fn handle_bytes_with_fds(&mut self, request: &[u8], fds: Vec<RawFd>) -> Vec<u8> {
        let response = match uds_control_transport::decode_command(request) {
            Ok(command) => self.service.handle(inject_fds(command, fds)),
            Err(error) => Err(ControlError::new(error.stage, error.message)),
        };
        uds_control_transport::encode_reply(&response)
    }

    pub fn serve_connection(&mut self, stream: &mut UnixStream) -> std::io::Result<()> {
        let (request, fds) = read_request_with_fds(stream)?;
        let reply = self.handle_bytes_with_fds(&request, fds);
        stream.write_all(&reply)?;
        Ok(())
    }

    pub fn service_mut(&mut self) -> &mut S {
        &mut self.service
    }
}

impl UdsControlConnection {
    pub fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            reply: None,
            written: usize::default(),
        }
    }

    pub fn raw_fd(&self) -> RawFd {
        self.stream.as_raw_fd()
    }

    pub fn try_progress<S>(&mut self, server: &mut UdsControlServer<S>) -> std::io::Result<bool>
    where
        S: ControlService,
    {
        if self.reply.is_none() {
            match read_request_with_fds(&self.stream) {
                Ok((request, _)) if request.is_empty() => return Ok(true),
                Ok((request, fds)) => {
                    self.reply = Some(server.handle_bytes_with_fds(&request, fds));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) => return Err(error),
            }
        }
        self.try_write_reply()
    }

    fn try_write_reply(&mut self) -> std::io::Result<bool> {
        let Some(reply) = self.reply.as_ref() else {
            return Ok(false);
        };
        while self.written < reply.len() {
            match self.stream.write(&reply[self.written..]) {
                Ok(0) => return Ok(true),
                Ok(written) => self.written = self.written.saturating_add(written),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) => return Err(error),
            }
        }
        Ok(true)
    }
}

fn inject_fds(mut command: ControlCommand, fds: Vec<RawFd>) -> ControlCommand {
    if let ControlCommand::RegisterSeccompListener(command) = &mut command {
        command.listener_fd = fds.into_iter().next();
    }
    command
}

fn read_request_with_fds(stream: &UnixStream) -> std::io::Result<(Vec<u8>, Vec<RawFd>)> {
    let mut request = vec![0_u8; request_buffer_bytes(stream)?];
    let mut iov = libc::iovec {
        iov_base: request.as_mut_ptr().cast(),
        iov_len: request.len(),
    };
    let control_len =
        unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as libc::c_uint) } as usize;
    let mut control = vec![0_u8; control_len];
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut iov;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = control.len();

    let received = unsafe { libc::recvmsg(stream.as_raw_fd(), &mut message, 0) };
    if received < 0 {
        return Err(std::io::Error::last_os_error());
    }
    request.truncate(received as usize);
    let fds = received_fds(&message);
    Ok((request, fds))
}

fn request_buffer_bytes(stream: &UnixStream) -> std::io::Result<usize> {
    let mut value = libc::c_int::default();
    let mut value_len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            (&mut value as *mut libc::c_int).cast(),
            &mut value_len,
        )
    };
    if result < 0 {
        return Err(std::io::Error::last_os_error());
    }
    usize::try_from(value).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("socket receive buffer size overflow: {error}"),
        )
    })
}

fn received_fds(message: &libc::msghdr) -> Vec<RawFd> {
    let mut fds = Vec::new();
    unsafe {
        let header = libc::CMSG_FIRSTHDR(message);
        if header.is_null()
            || (*header).cmsg_level != libc::SOL_SOCKET
            || (*header).cmsg_type != libc::SCM_RIGHTS
        {
            return fds;
        }
        let data_len = (*header)
            .cmsg_len
            .saturating_sub(libc::CMSG_LEN(0) as usize);
        let fd_count = data_len / std::mem::size_of::<RawFd>();
        let data = libc::CMSG_DATA(header).cast::<RawFd>();
        for index in 0..fd_count {
            fds.push(*data.add(index));
        }
    }
    fds
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use control_contract::command::{ControlCommand, DoctorCommand};
    use control_contract::reply::DoctorReply;
    use model_core::ids::RequestId;

    use super::*;

    const DOCTOR_REQUEST_ID: u64 = 7;

    struct DoctorService {
        calls: usize,
    }

    impl ControlService for DoctorService {
        fn handle(&mut self, command: ControlCommand) -> Result<ControlReply, ControlError> {
            match command {
                ControlCommand::Doctor(command) => {
                    assert_eq!(command.request_id, RequestId::new(DOCTOR_REQUEST_ID));
                    self.calls = self.calls.saturating_add(1);
                    Ok(ControlReply::Doctor(DoctorReply {
                        available_collectors: vec!["uds-test".to_string()],
                        loaded_policy_plugins: Vec::new(),
                        storage_ready: true,
                    }))
                }
                _ => Err(ControlError::new("unexpected_command", "expected doctor")),
            }
        }
    }

    #[test]
    fn nonblocking_connection_waits_for_request_then_replies() {
        let (mut client_stream, server_stream) = UnixStream::pair().unwrap();
        server_stream.set_nonblocking(true).unwrap();
        let mut connection = UdsControlConnection::new(server_stream);
        let mut server = UdsControlServer::new(DoctorService { calls: 0 });

        assert!(!connection.try_progress(&mut server).unwrap());
        assert_eq!(server.service_mut().calls, 0);

        let request =
            uds_control_transport::encode_command(&ControlCommand::Doctor(DoctorCommand {
                request_id: RequestId::new(DOCTOR_REQUEST_ID),
            }));
        client_stream.write_all(&request).unwrap();

        assert!(connection.try_progress(&mut server).unwrap());
        assert_eq!(server.service_mut().calls, 1);

        drop(connection);
        let mut reply = Vec::new();
        client_stream.read_to_end(&mut reply).unwrap();
        let decoded = uds_control_transport::decode_reply(&reply).unwrap();

        assert_eq!(
            decoded,
            Ok(ControlReply::Doctor(DoctorReply {
                available_collectors: vec!["uds-test".to_string()],
                loaded_policy_plugins: Vec::new(),
                storage_ready: true,
            }))
        );
    }
}
