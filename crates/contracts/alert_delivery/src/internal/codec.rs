use serde_json::Value;

use crate::{
    DeliveryCodecError, DeliverySeverity, DeliverySource, ForwardAlert, SandboxDeliverySource,
    SandboxProcessMarker,
};

use super::frame::{ATAP_HEADER_BYTES, AtapHeader, AtapMessageCode};
use super::message::{AtapMessage, Heartbeat, HeartbeatAck, ProducerHello, ProducerReject};
use super::payload::{PayloadCursor, PayloadWriter};

const MIN_13_DIGIT_TIMESTAMP: u64 = 1_000_000_000_000;
const MAX_13_DIGIT_TIMESTAMP: u64 = 9_999_999_999_999;
const SOURCE_TRACE: u8 = 1;
const SOURCE_SANDBOX: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AtapLimits {
    max_frame_bytes: usize,
    trace_id_max_bytes: usize,
    category_max_bytes: usize,
    description_max_bytes: usize,
    extras_max_bytes: usize,
}

impl AtapLimits {
    pub fn new(
        max_frame_bytes: usize,
        trace_id_max_bytes: usize,
        category_max_bytes: usize,
        description_max_bytes: usize,
        extras_max_bytes: usize,
    ) -> Result<Self, DeliveryCodecError> {
        let limits = Self {
            max_frame_bytes,
            trace_id_max_bytes,
            category_max_bytes,
            description_max_bytes,
            extras_max_bytes,
        };
        limits.validate()?;
        Ok(limits)
    }

    pub const fn max_frame_bytes(self) -> usize {
        self.max_frame_bytes
    }

    pub const fn trace_id_max_bytes(self) -> usize {
        self.trace_id_max_bytes
    }

    pub const fn category_max_bytes(self) -> usize {
        self.category_max_bytes
    }

    pub const fn description_max_bytes(self) -> usize {
        self.description_max_bytes
    }

    pub const fn extras_max_bytes(self) -> usize {
        self.extras_max_bytes
    }

    fn validate(self) -> Result<(), DeliveryCodecError> {
        if self.max_frame_bytes <= ATAP_HEADER_BYTES {
            return Err(DeliveryCodecError::new(
                "atap_limits",
                "max frame bytes must exceed the ATAP header size",
            ));
        }
        for (name, value, protocol_max) in [
            (
                "trace_id_max_bytes",
                self.trace_id_max_bytes,
                u16::MAX as usize,
            ),
            (
                "category_max_bytes",
                self.category_max_bytes,
                u16::MAX as usize,
            ),
            (
                "description_max_bytes",
                self.description_max_bytes,
                u16::MAX as usize,
            ),
            ("extras_max_bytes", self.extras_max_bytes, u32::MAX as usize),
        ] {
            if value == 0 || value > protocol_max {
                return Err(DeliveryCodecError::new(
                    "atap_limits",
                    format!("{name} must be between 1 and {protocol_max}"),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct AtapCodec {
    limits: AtapLimits,
}

impl AtapCodec {
    pub fn new(limits: AtapLimits) -> Self {
        Self { limits }
    }

    pub const fn limits(&self) -> AtapLimits {
        self.limits
    }

    pub fn encode(&self, message: &AtapMessage) -> Result<Vec<u8>, DeliveryCodecError> {
        let (code, payload) = match message {
            AtapMessage::ProducerHello(hello) => {
                if hello.daemon_pid == 0 {
                    return Err(DeliveryCodecError::new(
                        "atap_encode",
                        "daemon PID must not be zero",
                    ));
                }
                (
                    AtapMessageCode::ProducerHello,
                    hello.daemon_pid.to_be_bytes().to_vec(),
                )
            }
            AtapMessage::ProducerWelcome => (AtapMessageCode::ProducerWelcome, Vec::new()),
            AtapMessage::ProducerReject(reject) => {
                let mut writer = PayloadWriter::new();
                writer.write_required_u16_string("reject code", &reject.code, u16::MAX as usize)?;
                (AtapMessageCode::ProducerReject, writer.finish())
            }
            AtapMessage::ForwardAlert(alert) => {
                (AtapMessageCode::ForwardAlert, self.encode_alert(alert)?)
            }
            AtapMessage::Heartbeat(heartbeat) => (
                AtapMessageCode::Heartbeat,
                heartbeat.nonce.to_be_bytes().to_vec(),
            ),
            AtapMessage::HeartbeatAck(heartbeat) => (
                AtapMessageCode::HeartbeatAck,
                heartbeat.nonce.to_be_bytes().to_vec(),
            ),
        };
        self.encode_frame(code, payload)
    }

    pub fn decode(&self, frame: &[u8]) -> Result<AtapMessage, DeliveryCodecError> {
        if frame.len() < ATAP_HEADER_BYTES {
            return Err(DeliveryCodecError::new(
                "atap_decode",
                "ATAP frame is shorter than its header",
            ));
        }
        let frame_length = self.frame_length(&frame[..ATAP_HEADER_BYTES])?;
        if frame.len() != frame_length {
            return Err(DeliveryCodecError::new(
                "atap_decode",
                format!(
                    "ATAP frame length mismatch: expected {frame_length}, received {}",
                    frame.len()
                ),
            ));
        }
        let header_bytes: &[u8; ATAP_HEADER_BYTES] = frame[..ATAP_HEADER_BYTES]
            .try_into()
            .expect("checked header size");
        let header = AtapHeader::decode(header_bytes)?;
        let payload = &frame[ATAP_HEADER_BYTES..];
        self.decode_payload(header.code, payload)
    }

    pub(crate) fn frame_length(&self, header: &[u8]) -> Result<usize, DeliveryCodecError> {
        let bytes: &[u8; ATAP_HEADER_BYTES] = header.try_into().map_err(|_| {
            DeliveryCodecError::new("atap_header", "ATAP header must contain exactly 12 bytes")
        })?;
        let decoded = AtapHeader::decode(bytes)?;
        let frame_length = ATAP_HEADER_BYTES
            .checked_add(decoded.payload_length as usize)
            .ok_or_else(|| DeliveryCodecError::new("atap_header", "frame length overflow"))?;
        if frame_length > self.limits.max_frame_bytes {
            return Err(DeliveryCodecError::new(
                "atap_header",
                format!(
                    "frame length {frame_length} exceeds configured limit {}",
                    self.limits.max_frame_bytes
                ),
            ));
        }
        Ok(frame_length)
    }

    fn encode_frame(
        &self,
        code: AtapMessageCode,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, DeliveryCodecError> {
        let payload_length = u32::try_from(payload.len()).map_err(|_| {
            DeliveryCodecError::new("atap_encode", "payload length does not fit u32")
        })?;
        let frame_length = ATAP_HEADER_BYTES
            .checked_add(payload.len())
            .ok_or_else(|| DeliveryCodecError::new("atap_encode", "frame length overflow"))?;
        if frame_length > self.limits.max_frame_bytes {
            return Err(DeliveryCodecError::new(
                "atap_encode",
                format!(
                    "frame length {frame_length} exceeds configured limit {}",
                    self.limits.max_frame_bytes
                ),
            ));
        }
        let header = AtapHeader {
            code,
            payload_length,
        }
        .encode();
        let mut frame = Vec::with_capacity(frame_length);
        frame.extend_from_slice(&header);
        frame.extend_from_slice(&payload);
        Ok(frame)
    }

    fn encode_alert(&self, alert: &ForwardAlert) -> Result<Vec<u8>, DeliveryCodecError> {
        validate_timestamp(alert.detected_at_ms, "atap_encode")?;
        let extras = serde_json::to_vec(&alert.extras).map_err(|error| {
            DeliveryCodecError::new("atap_encode", format!("serialize extras JSON: {error}"))
        })?;
        if extras.len() > self.limits.extras_max_bytes {
            return Err(DeliveryCodecError::new(
                "atap_encode",
                "extras exceeds configured byte limit",
            ));
        }
        let mut writer = PayloadWriter::new();
        writer.write_u64(alert.detected_at_ms);
        writer.write_u8(alert.severity.code());
        self.encode_source(&mut writer, &alert.source)?;
        writer.write_required_u16_string(
            "category",
            &alert.category,
            self.limits.category_max_bytes,
        )?;
        match alert.description.as_deref() {
            Some(description) => {
                writer.write_u8(1);
                writer.write_u16_string(
                    "description",
                    description,
                    self.limits.description_max_bytes,
                )?;
            }
            None => writer.write_u8(0),
        }
        writer.write_u32_bytes("extras", &extras)?;
        Ok(writer.finish())
    }

    fn encode_source(
        &self,
        writer: &mut PayloadWriter,
        source: &DeliverySource,
    ) -> Result<(), DeliveryCodecError> {
        match source {
            DeliverySource::Trace { trid } => {
                writer.write_u8(SOURCE_TRACE);
                writer.write_required_u16_string("trace ID", trid, self.limits.trace_id_max_bytes)
            }
            DeliverySource::Sandbox(source) => {
                validate_sandbox_source(source, "atap_encode")?;
                writer.write_u8(SOURCE_SANDBOX);
                writer.write_u32(source.gateway_id);
                writer.write_u32(source.sb_id);
                writer.write_fixed(&source.boot_id);
                match &source.process {
                    Some(process) => {
                        writer.write_u8(1);
                        writer.write_u32(process.pid);
                        writer.write_u64(process.start_time_ticks);
                        writer.write_fixed(&process.executable_name);
                    }
                    None => writer.write_u8(0),
                }
                Ok(())
            }
        }
    }

    fn decode_payload(
        &self,
        code: AtapMessageCode,
        payload: &[u8],
    ) -> Result<AtapMessage, DeliveryCodecError> {
        match code {
            AtapMessageCode::ProducerHello => {
                let mut cursor = PayloadCursor::new(payload);
                let daemon_pid = cursor.read_u32("daemon PID")?;
                cursor.finish()?;
                if daemon_pid == 0 {
                    return Err(DeliveryCodecError::new(
                        "atap_decode",
                        "daemon PID must not be zero",
                    ));
                }
                Ok(AtapMessage::ProducerHello(ProducerHello { daemon_pid }))
            }
            AtapMessageCode::ProducerWelcome => {
                require_empty_payload(payload, "ProducerWelcome")?;
                Ok(AtapMessage::ProducerWelcome)
            }
            AtapMessageCode::ProducerReject => {
                let mut cursor = PayloadCursor::new(payload);
                let code = cursor.read_required_u16_string("reject code", u16::MAX as usize)?;
                cursor.finish()?;
                Ok(AtapMessage::ProducerReject(ProducerReject { code }))
            }
            AtapMessageCode::ForwardAlert => {
                self.decode_alert(payload).map(AtapMessage::ForwardAlert)
            }
            AtapMessageCode::Heartbeat => {
                decode_nonce(payload).map(|nonce| AtapMessage::Heartbeat(Heartbeat { nonce }))
            }
            AtapMessageCode::HeartbeatAck => {
                decode_nonce(payload).map(|nonce| AtapMessage::HeartbeatAck(HeartbeatAck { nonce }))
            }
        }
    }

    fn decode_alert(&self, payload: &[u8]) -> Result<ForwardAlert, DeliveryCodecError> {
        let mut cursor = PayloadCursor::new(payload);
        let detected_at_ms = cursor.read_u64("detected timestamp")?;
        validate_timestamp(detected_at_ms, "atap_decode")?;
        let severity_code = cursor.read_u8("severity")?;
        let severity = DeliverySeverity::from_code(severity_code).ok_or_else(|| {
            DeliveryCodecError::new(
                "atap_decode",
                format!("unknown delivery severity code {severity_code}"),
            )
        })?;
        let source = self.decode_source(&mut cursor)?;
        let category =
            cursor.read_required_u16_string("category", self.limits.category_max_bytes)?;
        let description = match cursor.read_u8("description presence")? {
            0 => None,
            1 => Some(cursor.read_u16_string("description", self.limits.description_max_bytes)?),
            other => {
                return Err(DeliveryCodecError::new(
                    "atap_decode",
                    format!("invalid description presence marker {other}"),
                ));
            }
        };
        let extras_bytes = cursor.read_u32_bytes("extras", self.limits.extras_max_bytes)?;
        cursor.finish()?;
        let extras_value = serde_json::from_slice::<Value>(extras_bytes).map_err(|error| {
            DeliveryCodecError::new("atap_decode", format!("decode extras JSON: {error}"))
        })?;
        let extras = match extras_value {
            Value::Object(extras) => extras,
            _ => {
                return Err(DeliveryCodecError::new(
                    "atap_decode",
                    "extras JSON must be an object",
                ));
            }
        };
        Ok(ForwardAlert {
            detected_at_ms,
            severity,
            source,
            category,
            description,
            extras,
        })
    }

    fn decode_source(
        &self,
        cursor: &mut PayloadCursor<'_>,
    ) -> Result<DeliverySource, DeliveryCodecError> {
        match cursor.read_u8("source kind")? {
            SOURCE_TRACE => cursor
                .read_required_u16_string("trace ID", self.limits.trace_id_max_bytes)
                .map(|trid| DeliverySource::Trace { trid }),
            SOURCE_SANDBOX => {
                let source = SandboxDeliverySource {
                    gateway_id: cursor.read_u32("sandbox gateway ID")?,
                    sb_id: cursor.read_u32("sandbox ID")?,
                    boot_id: cursor.read_fixed("sandbox boot ID")?,
                    process: match cursor.read_u8("sandbox process presence")? {
                        0 => None,
                        1 => Some(SandboxProcessMarker {
                            pid: cursor.read_u32("sandbox process PID")?,
                            start_time_ticks: cursor.read_u64("sandbox process start ticks")?,
                            executable_name: cursor.read_fixed("sandbox executable name")?,
                        }),
                        other => {
                            return Err(DeliveryCodecError::new(
                                "atap_decode",
                                format!("invalid sandbox process presence marker {other}"),
                            ));
                        }
                    },
                };
                validate_sandbox_source(&source, "atap_decode")?;
                Ok(DeliverySource::Sandbox(source))
            }
            other => Err(DeliveryCodecError::new(
                "atap_decode",
                format!("unknown alert source kind {other}"),
            )),
        }
    }
}

fn validate_sandbox_source(
    source: &SandboxDeliverySource,
    stage: &'static str,
) -> Result<(), DeliveryCodecError> {
    if source.gateway_id == 0 || source.sb_id == 0 {
        return Err(DeliveryCodecError::new(
            stage,
            "sandbox gateway ID and sandbox ID must not be zero",
        ));
    }
    if source.boot_id == [0; 16] {
        return Err(DeliveryCodecError::new(
            stage,
            "sandbox boot ID must not be zero",
        ));
    }
    if source
        .process
        .as_ref()
        .is_some_and(|process| process.pid == 0 || process.start_time_ticks == 0)
    {
        return Err(DeliveryCodecError::new(
            stage,
            "sandbox process PID and start ticks must not be zero",
        ));
    }
    Ok(())
}

fn decode_nonce(payload: &[u8]) -> Result<u64, DeliveryCodecError> {
    let mut cursor = PayloadCursor::new(payload);
    let nonce = cursor.read_u64("heartbeat nonce")?;
    cursor.finish()?;
    Ok(nonce)
}

fn require_empty_payload(payload: &[u8], message: &str) -> Result<(), DeliveryCodecError> {
    if payload.is_empty() {
        Ok(())
    } else {
        Err(DeliveryCodecError::new(
            "atap_decode",
            format!("{message} payload must be empty"),
        ))
    }
}

fn validate_timestamp(value: u64, stage: &'static str) -> Result<(), DeliveryCodecError> {
    if (MIN_13_DIGIT_TIMESTAMP..=MAX_13_DIGIT_TIMESTAMP).contains(&value) {
        Ok(())
    } else {
        Err(DeliveryCodecError::new(
            stage,
            "detected timestamp must contain 13 decimal digits",
        ))
    }
}
