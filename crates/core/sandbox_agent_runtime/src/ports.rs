use std::io::{self, Read, Write};

use sandbox_observation::{GuestResourceSnapshot, ProcessIoCounters};

pub trait ProcessIoSource: Send + 'static {
    fn poll(&mut self) -> io::Result<Vec<ProcessIoCounters>>;
}

pub trait GuestResourceSource: Send + 'static {
    fn sample(&mut self) -> io::Result<GuestResourceSnapshot>;
}

pub trait SandboxConnection: Read + Write + Send + 'static {}

impl<T> SandboxConnection for T where T: Read + Write + Send + 'static {}

pub trait SandboxTransport: Send + Sync + 'static {
    fn connect(&self) -> io::Result<Box<dyn SandboxConnection>>;
}
