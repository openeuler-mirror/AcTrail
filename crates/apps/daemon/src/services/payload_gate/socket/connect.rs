//! Minimal client-side HTTP/1 CONNECT tunnel tracking.

use model_core::payload::PayloadDirection;

const HTTP_HEAD_END: &[u8] = b"\r\n\r\n";

pub(super) struct ClientConnectTunnelGate {
    max_head_bytes: usize,
    state: ClientConnectState,
}

impl ClientConnectTunnelGate {
    pub(super) fn awaiting_response(max_head_bytes: u64) -> Result<Self, String> {
        Ok(Self {
            max_head_bytes: usize::try_from(max_head_bytes)
                .map_err(|error| format!("socket HTTP head limit overflow: {error}"))?,
            state: ClientConnectState::AwaitingResponse(Vec::new()),
        })
    }

    pub(super) fn observe(
        &mut self,
        direction: PayloadDirection,
        bytes: &[u8],
    ) -> ConnectTunnelDecision {
        match &mut self.state {
            ClientConnectState::Inactive => ConnectTunnelDecision::Admit,
            ClientConnectState::AwaitingResponse(buffer) => {
                if direction != PayloadDirection::Inbound {
                    return ConnectTunnelDecision::Admit;
                }
                let outcome = ConnectResponseHead::observe(buffer, bytes, self.max_head_bytes);
                match outcome {
                    ConnectResponseOutcome::NeedMore => ConnectTunnelDecision::Admit,
                    ConnectResponseOutcome::Established {
                        admitted_prefix_len,
                    } => {
                        self.state = ClientConnectState::Established;
                        ConnectTunnelDecision::Established {
                            admitted_prefix_len,
                        }
                    }
                    ConnectResponseOutcome::Retry => {
                        self.state = ClientConnectState::AwaitingRetry(Vec::new());
                        ConnectTunnelDecision::Admit
                    }
                    ConnectResponseOutcome::StopTracking => {
                        self.state = ClientConnectState::Inactive;
                        ConnectTunnelDecision::Admit
                    }
                }
            }
            ClientConnectState::AwaitingRetry(buffer) => {
                if direction != PayloadDirection::Outbound {
                    return ConnectTunnelDecision::Admit;
                }
                match ConnectRequestLine::observe(buffer, bytes, self.max_head_bytes) {
                    ConnectRequestOutcome::NeedMore => ConnectTunnelDecision::Admit,
                    ConnectRequestOutcome::Connect => {
                        self.state = ClientConnectState::AwaitingResponse(Vec::new());
                        ConnectTunnelDecision::Admit
                    }
                    ConnectRequestOutcome::Other => {
                        self.state = ClientConnectState::Inactive;
                        ConnectTunnelDecision::Admit
                    }
                }
            }
            ClientConnectState::Established => ConnectTunnelDecision::Established {
                admitted_prefix_len: 0,
            },
        }
    }
}

enum ClientConnectState {
    Inactive,
    AwaitingResponse(Vec<u8>),
    AwaitingRetry(Vec<u8>),
    Established,
}

pub(super) enum ConnectTunnelDecision {
    Admit,
    Established { admitted_prefix_len: usize },
}

struct ConnectResponseHead;

impl ConnectResponseHead {
    fn observe(
        buffer: &mut Vec<u8>,
        bytes: &[u8],
        max_head_bytes: usize,
    ) -> ConnectResponseOutcome {
        let previous_len = buffer.len();
        let Some(remaining) = max_head_bytes.checked_sub(previous_len) else {
            return ConnectResponseOutcome::StopTracking;
        };
        let copy_len = remaining.min(bytes.len());
        buffer.extend_from_slice(&bytes[..copy_len]);

        let mut offset = 0;
        loop {
            let Some(relative_end) = buffer[offset..]
                .windows(HTTP_HEAD_END.len())
                .position(|window| window == HTTP_HEAD_END)
            else {
                return if copy_len < bytes.len() || buffer.len() >= max_head_bytes {
                    ConnectResponseOutcome::StopTracking
                } else {
                    ConnectResponseOutcome::NeedMore
                };
            };
            let head_end = offset + relative_end + HTTP_HEAD_END.len();
            let Some(status) = Self::status_code(&buffer[offset..head_end]) else {
                return ConnectResponseOutcome::StopTracking;
            };
            match status {
                100..=199 => offset = head_end,
                200..=299 => {
                    return ConnectResponseOutcome::Established {
                        admitted_prefix_len: head_end.saturating_sub(previous_len).min(bytes.len()),
                    };
                }
                _ => return ConnectResponseOutcome::Retry,
            }
        }
    }

    fn status_code(head: &[u8]) -> Option<u16> {
        let line_end = head.windows(2).position(|window| window == b"\r\n")?;
        let line = std::str::from_utf8(&head[..line_end]).ok()?;
        let mut parts = line.split_whitespace();
        let version = parts.next()?;
        let status = parts.next()?;
        if !matches!(version, "HTTP/1.0" | "HTTP/1.1")
            || status.len() != 3
            || !status.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        status.parse().ok()
    }
}

enum ConnectResponseOutcome {
    NeedMore,
    Established { admitted_prefix_len: usize },
    Retry,
    StopTracking,
}

struct ConnectRequestLine;

impl ConnectRequestLine {
    fn observe(buffer: &mut Vec<u8>, bytes: &[u8], max_head_bytes: usize) -> ConnectRequestOutcome {
        let Some(remaining) = max_head_bytes.checked_sub(buffer.len()) else {
            return ConnectRequestOutcome::Other;
        };
        let copy_len = remaining.min(bytes.len());
        buffer.extend_from_slice(&bytes[..copy_len]);
        let Some(line_end) = buffer.iter().position(|byte| *byte == b'\n') else {
            return if copy_len < bytes.len() || buffer.len() >= max_head_bytes {
                ConnectRequestOutcome::Other
            } else {
                ConnectRequestOutcome::NeedMore
            };
        };
        let Ok(line) = std::str::from_utf8(&buffer[..line_end]) else {
            return ConnectRequestOutcome::Other;
        };
        let mut parts = line.trim_end_matches('\r').split_whitespace();
        let method = parts.next();
        let authority = parts.next();
        let version = parts.next();
        if method == Some("CONNECT")
            && authority.is_some_and(|value| !value.is_empty())
            && version.is_some_and(|value| matches!(value, "HTTP/1.0" | "HTTP/1.1"))
            && parts.next().is_none()
        {
            ConnectRequestOutcome::Connect
        } else {
            ConnectRequestOutcome::Other
        }
    }
}

enum ConnectRequestOutcome {
    NeedMore,
    Connect,
    Other,
}
