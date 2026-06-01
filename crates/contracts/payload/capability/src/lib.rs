//! Payload capability contracts and request modes.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadCapability {
    PlaintextHttp,
    PlaintextWebSocket,
    Stdin,
    IpcPayload,
}
