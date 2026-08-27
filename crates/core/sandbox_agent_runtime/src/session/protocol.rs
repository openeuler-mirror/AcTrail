use std::io;

use sandbox_observation::{Observation, ObservationBatch};
use sandbox_vsock_contract::{Frame, FrameCode, FrameDecoder, ObservationBatchCodec};

use crate::SandboxConnection;

pub(super) struct SessionProtocol {
    codec: ObservationBatchCodec,
}

impl SessionProtocol {
    pub(super) fn new() -> Self {
        Self {
            codec: ObservationBatchCodec,
        }
    }

    pub(super) fn handshake(&self, connection: &mut dyn SandboxConnection) -> io::Result<u32> {
        self.write_frame(
            connection,
            &Frame::new(FrameCode::SbHello, Vec::new())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
        )?;
        let welcome = self.read_frame(connection)?;
        if welcome.code != FrameCode::SbWelcome {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "gateway did not return SbWelcome",
            ));
        }
        let sb_id = welcome
            .decode_numeric_id()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if sb_id == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "gateway assigned reserved SB ID zero",
            ));
        }
        Ok(sb_id)
    }

    pub(super) fn send_batch(
        &self,
        connection: &mut dyn SandboxConnection,
        sequence: u64,
        observations: &mut Vec<Observation>,
    ) -> io::Result<()> {
        let mut batch = ObservationBatch::new(sequence, std::mem::take(observations));
        let encoded = self.codec.encode(&batch);
        *observations = std::mem::take(&mut batch.observations);
        let payload = encoded.map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let frame = Frame::new(FrameCode::ObservationBatch, payload)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        self.write_frame(connection, &frame)
    }

    pub(super) fn send_heartbeat(&self, connection: &mut dyn SandboxConnection) -> io::Result<()> {
        let frame = Frame::new(FrameCode::Heartbeat, Vec::new()).expect("fixed heartbeat frame");
        self.write_frame(connection, &frame)
    }

    fn write_frame(&self, connection: &mut dyn SandboxConnection, frame: &Frame) -> io::Result<()> {
        let bytes = frame
            .encode()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        connection.write_all(&bytes)
    }

    fn read_frame(&self, connection: &mut dyn SandboxConnection) -> io::Result<Frame> {
        let mut decoder = FrameDecoder::with_capacity(1024);
        let mut buffer = [0_u8; 1024];
        loop {
            let count = connection.read(&mut buffer)?;
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "gateway closed during SB handshake",
                ));
            }
            decoder.push(&buffer[..count]);
            if let Some(frame) = decoder
                .next_frame()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
            {
                return Ok(frame);
            }
        }
    }
}
