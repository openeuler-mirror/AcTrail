use std::io;

use vsock_gateway_runtime::GatewayRuntime;

use crate::GatewayAppConfig;

pub struct GatewayBootstrap;

impl GatewayBootstrap {
    pub fn start(config: GatewayAppConfig) -> io::Result<GatewayRuntime> {
        GatewayRuntime::start(config.into_runtime()?)
    }
}
