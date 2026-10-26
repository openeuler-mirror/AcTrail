use super::fields::{FieldReader, FieldWriter};
use super::header::{FrameHeader, FrameKind};
use crate::{
    DecisionEvent, PayloadEvent, PlanLookupRequest, PlanLookupResponse, RuntimePlanDescriptor,
    SummaryEvent, SyncError, SyncEvent, SyncResult,
};
use std::io::{Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use tls_payload_core::PayloadDirection;
use tls_probe_point_finder::{BinaryIdentity, BinaryIdentityTypeCode};

#[derive(Debug)]
pub enum SyncMessage {
    Event(SyncEvent),
    PlanLookup(PlanLookupRequest),
    PlanResponse(PlanLookupResponse),
}

pub struct FrameCodec;
impl FrameCodec {
    pub const HEADER_LEN: usize = FrameHeader::LEN;
    pub fn parse_header(bytes: &[u8], max_frame_bytes: usize) -> SyncResult<Option<FrameHeader>> {
        FrameHeader::parse(bytes, max_frame_bytes)
    }
    pub fn decode(header: FrameHeader, body: &[u8]) -> SyncResult<SyncMessage> {
        if body.len() != header.body_len() {
            return Err(SyncError::new("TLS frame body length mismatch"));
        }
        let mut fields = FieldReader::new(body);
        let message = match header.kind() {
            FrameKind::Payload | FrameKind::Decision | FrameKind::Summary => {
                SyncMessage::Event(Self::read_event(&mut fields, header.kind())?)
            }
            FrameKind::PlanLookup => SyncMessage::PlanLookup(PlanLookupRequest {
                binary: Self::read_path(&mut fields)?,
            }),
            FrameKind::PlanFound => {
                SyncMessage::PlanResponse(PlanLookupResponse::Found(RuntimePlanDescriptor {
                    target: Self::read_path(&mut fields)?,
                    target_identity: Self::read_identity(&mut fields)?,
                    binary: Self::read_path(&mut fields)?,
                    binary_identity: Self::read_identity(&mut fields)?,
                    provider: fields.text()?,
                    points: fields.text()?,
                }))
            }
            FrameKind::PlanUnavailable => {
                SyncMessage::PlanResponse(PlanLookupResponse::Unsupported {
                    reason: fields.text()?,
                })
            }
        };
        fields.finish()?;
        Ok(message)
    }
    pub fn write_event(
        writer: &mut impl Write,
        event: &SyncEvent,
        max_frame_bytes: usize,
    ) -> SyncResult<()> {
        let kind = match event {
            SyncEvent::Payload(_) => FrameKind::Payload,
            SyncEvent::Decision(_) => FrameKind::Decision,
            SyncEvent::Summary(_) => FrameKind::Summary,
        };
        let mut counter = FieldWriter::new(std::io::sink());
        Self::event_fields(&mut counter, event)?;
        let header = FrameHeader::new(kind, counter.len(), max_frame_bytes)?;
        writer.write_all(&header.encode())?;
        Self::event_fields(&mut FieldWriter::new(writer), event)
    }
    pub fn write_lookup_request(
        writer: &mut impl Write,
        request: &PlanLookupRequest,
        max_frame_bytes: usize,
    ) -> SyncResult<()> {
        let bytes = request.binary.as_os_str().as_bytes();
        let mut counter = FieldWriter::new(std::io::sink());
        counter.bytes(bytes)?;
        let header = FrameHeader::new(FrameKind::PlanLookup, counter.len(), max_frame_bytes)?;
        let mut frame = Self::control_frame(header)?;
        FieldWriter::new(&mut frame).bytes(bytes)?;
        writer.write_all(&frame)?;
        Ok(())
    }
    pub fn write_lookup_response(
        writer: &mut impl Write,
        response: &PlanLookupResponse,
        max_frame_bytes: usize,
    ) -> SyncResult<()> {
        let kind = match response {
            PlanLookupResponse::Found(_) => FrameKind::PlanFound,
            PlanLookupResponse::Unsupported { .. } => FrameKind::PlanUnavailable,
        };
        let mut counter = FieldWriter::new(std::io::sink());
        Self::response_fields(&mut counter, response)?;
        let header = FrameHeader::new(kind, counter.len(), max_frame_bytes)?;
        let mut frame = Self::control_frame(header)?;
        Self::response_fields(&mut FieldWriter::new(&mut frame), response)?;
        writer.write_all(&frame)?;
        Ok(())
    }
    fn control_frame(header: FrameHeader) -> SyncResult<Vec<u8>> {
        let mut frame = Vec::new();
        frame
            .try_reserve_exact(header.frame_len())
            .map_err(|_| SyncError::new("TLS control frame allocation failed"))?;
        frame.extend_from_slice(&header.encode());
        Ok(frame)
    }
    pub fn read_lookup_response(
        reader: &mut impl Read,
        max_frame_bytes: usize,
    ) -> SyncResult<PlanLookupResponse> {
        let mut bytes = [0; FrameHeader::LEN];
        reader.read_exact(&mut bytes)?;
        let header = Self::parse_header(&bytes, max_frame_bytes)?.expect("complete fixed header");
        if !matches!(
            header.kind(),
            FrameKind::PlanFound | FrameKind::PlanUnavailable
        ) {
            return Err(SyncError::new("expected TLS plan response frame"));
        }
        let mut body = Vec::new();
        body.try_reserve_exact(header.body_len())
            .map_err(|_| SyncError::new("TLS plan response allocation failed"))?;
        body.resize(header.body_len(), 0);
        reader.read_exact(&mut body)?;
        match Self::decode(header, &body)? {
            SyncMessage::PlanResponse(response) => Ok(response),
            _ => Err(SyncError::new("expected TLS plan response")),
        }
    }
    fn event_fields<W: Write>(fields: &mut FieldWriter<W>, event: &SyncEvent) -> SyncResult<()> {
        let (trace_id, pid, start_time, namespace, direction, provider, symbol, key, sequence) =
            match event {
                SyncEvent::Payload(e) => (
                    e.trace_id,
                    e.pid,
                    e.start_time_ticks,
                    &e.pid_namespace,
                    e.direction,
                    &e.provider,
                    &e.symbol,
                    e.stream_key,
                    e.sequence,
                ),
                SyncEvent::Decision(e) => (
                    e.trace_id,
                    e.pid,
                    e.start_time_ticks,
                    &e.pid_namespace,
                    e.direction,
                    &e.provider,
                    &e.symbol,
                    e.stream_key,
                    e.sequence,
                ),
                SyncEvent::Summary(e) => (
                    e.trace_id,
                    e.pid,
                    e.start_time_ticks,
                    &e.pid_namespace,
                    e.direction,
                    &e.provider,
                    &e.symbol,
                    e.stream_key,
                    e.sequence,
                ),
            };
        fields.u64(trace_id)?;
        fields.u32(pid)?;
        fields.u64(start_time)?;
        fields.text(namespace)?;
        fields.u8(match direction {
            PayloadDirection::Outbound => 0,
            PayloadDirection::Inbound => 1,
        })?;
        fields.text(provider)?;
        fields.text(symbol)?;
        fields.u64(key)?;
        fields.u64(sequence)?;
        match event {
            SyncEvent::Payload(e) => fields.bytes(&e.bytes),
            SyncEvent::Decision(e) => {
                fields.text(&e.action)?;
                fields.text(&e.reason)
            }
            SyncEvent::Summary(e) => {
                fields.u64(e.observed_size)?;
                fields.u64(e.emitted_size)?;
                fields.text(&e.reason)?;
                fields.text(&e.protocol_hint)?;
                fields.bytes(&e.bytes)
            }
        }
    }
    fn read_event(fields: &mut FieldReader<'_>, kind: FrameKind) -> SyncResult<SyncEvent> {
        let trace_id = fields.u64()?;
        let pid = fields.u32()?;
        let start_time_ticks = fields.u64()?;
        let pid_namespace = fields.text()?;
        let direction = match fields.u8()? {
            0 => PayloadDirection::Outbound,
            1 => PayloadDirection::Inbound,
            _ => return Err(SyncError::new("invalid TLS payload direction")),
        };
        let provider = fields.text()?;
        let symbol = fields.text()?;
        let stream_key = fields.u64()?;
        let sequence = fields.u64()?;
        Ok(match kind {
            FrameKind::Payload => SyncEvent::Payload(PayloadEvent {
                trace_id,
                pid,
                start_time_ticks,
                pid_namespace,
                direction,
                provider,
                symbol,
                stream_key,
                sequence,
                bytes: fields.bytes()?.to_vec(),
            }),
            FrameKind::Decision => SyncEvent::Decision(DecisionEvent {
                trace_id,
                pid,
                start_time_ticks,
                pid_namespace,
                direction,
                provider,
                symbol,
                stream_key,
                sequence,
                action: fields.text()?,
                reason: fields.text()?,
            }),
            FrameKind::Summary => SyncEvent::Summary(SummaryEvent {
                trace_id,
                pid,
                start_time_ticks,
                pid_namespace,
                direction,
                provider,
                symbol,
                stream_key,
                sequence,
                observed_size: fields.u64()?,
                emitted_size: fields.u64()?,
                reason: fields.text()?,
                protocol_hint: fields.text()?,
                bytes: fields.bytes()?.to_vec(),
            }),
            _ => return Err(SyncError::new("expected TLS event frame")),
        })
    }
    fn response_fields<W: Write>(
        fields: &mut FieldWriter<W>,
        response: &PlanLookupResponse,
    ) -> SyncResult<()> {
        match response {
            PlanLookupResponse::Unsupported { reason } => fields.text(reason),
            PlanLookupResponse::Found(plan) => {
                fields.bytes(plan.target.as_os_str().as_bytes())?;
                fields.u16(plan.target_identity.identity_type_code.code())?;
                fields.text(&plan.target_identity.identity)?;
                fields.bytes(plan.binary.as_os_str().as_bytes())?;
                fields.u16(plan.binary_identity.identity_type_code.code())?;
                fields.text(&plan.binary_identity.identity)?;
                fields.text(&plan.provider)?;
                fields.text(&plan.points)
            }
        }
    }
    fn read_path(fields: &mut FieldReader<'_>) -> SyncResult<PathBuf> {
        Ok(PathBuf::from(std::ffi::OsString::from_vec(
            fields.bytes()?.to_vec(),
        )))
    }
    fn read_identity(fields: &mut FieldReader<'_>) -> SyncResult<BinaryIdentity> {
        let code = BinaryIdentityTypeCode::parse(fields.u16()?)
            .map_err(|e| SyncError::new(e.to_string()))?;
        BinaryIdentity::try_new(code, fields.text()?).map_err(|e| SyncError::new(e.to_string()))
    }
}
