//! Application-owned control loop assembly.

use uds_control_server::{ControlService, UdsControlServer};

pub fn handle_request<S>(server: &mut UdsControlServer<S>, request: &[u8]) -> Vec<u8>
where
    S: ControlService,
{
    server.handle_bytes(request)
}
