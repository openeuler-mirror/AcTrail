use std::io::{self, Read, Write};

use sandbox_control::SandboxEndpoint;
use sandbox_observation::{GuestResourceSnapshot, Observation};

pub trait ProcessIoSource: Send + 'static {
    fn establish_baseline(&mut self) -> io::Result<()>;

    fn activate_publication(&mut self, generation: u64) -> io::Result<()>;

    fn poll(&mut self) -> io::Result<Vec<Observation>>;
}

pub trait GuestResourceSource: Send + 'static {
    fn sample(&mut self) -> io::Result<GuestResourceSnapshot>;
}

pub trait SandboxConnection: Read + Write + Send + 'static {}

impl<T> SandboxConnection for T where T: Read + Write + Send + 'static {}

/// Creates one data connection for a runtime-injected Guest VSOCK endpoint.
pub trait SandboxTransportFactory: Send + Sync + 'static {
    fn connect(&self, endpoint: SandboxEndpoint) -> io::Result<Box<dyn SandboxConnection>>;
}
