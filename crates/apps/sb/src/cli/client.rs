use std::io;

use sandbox_control::{
    SandboxConnectCommand, SandboxConnectResponse, SandboxControlCommand, SandboxControlResponse,
};
use sandbox_control_uds::{
    SandboxControlCodec, SandboxControlUdsClient, SandboxControlUdsClientConfig,
};

use super::SbConnectInvocation;

pub(super) struct SandboxConnectClient {
    client: SandboxControlUdsClient,
    command: SandboxControlCommand,
}

impl SandboxConnectClient {
    pub(super) fn new(invocation: SbConnectInvocation) -> io::Result<Self> {
        let config = SandboxControlUdsClientConfig::new(
            invocation.control_socket,
            invocation.request_timeout,
        )
        .map_err(io::Error::other)?;
        let codec =
            SandboxControlCodec::new(invocation.max_frame_bytes).map_err(io::Error::other)?;
        let command =
            SandboxControlCommand::Connect(SandboxConnectCommand::new(invocation.endpoint));
        Ok(Self {
            client: SandboxControlUdsClient::new(config, codec),
            command,
        })
    }

    pub(super) fn connect(&self) -> io::Result<SandboxConnectResponse> {
        match self.client.send(&self.command).map_err(io::Error::other)? {
            SandboxControlResponse::Connect(response) => Ok(response),
            SandboxControlResponse::Rejected(rejection) => Err(io::Error::other(format!(
                "sandbox connect rejected ({:?}): {}",
                rejection.code(),
                rejection.message()
            ))),
        }
    }
}
