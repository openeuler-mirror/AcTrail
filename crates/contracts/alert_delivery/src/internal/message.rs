use crate::ForwardAlert;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProducerHello {
    pub daemon_pid: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProducerReject {
    pub code: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Heartbeat {
    pub nonce: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeartbeatAck {
    pub nonce: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AtapMessage {
    ProducerHello(ProducerHello),
    ProducerWelcome,
    ProducerReject(ProducerReject),
    ForwardAlert(ForwardAlert),
    Heartbeat(Heartbeat),
    HeartbeatAck(HeartbeatAck),
}
