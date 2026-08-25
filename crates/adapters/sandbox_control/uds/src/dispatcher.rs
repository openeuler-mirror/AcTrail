//! Fixed single-worker dispatcher that keeps service execution outside the poll owner.

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, channel, sync_channel};
use std::thread::{self, JoinHandle};

use sandbox_control::{SandboxControlCommand, SandboxControlPort, SandboxControlResponse};

use crate::{SandboxControlUdsError, SandboxControlUdsStage};

pub(crate) struct DispatchResult {
    pub(crate) connection_id: u64,
    pub(crate) response: SandboxControlResponse,
}

struct DispatchRequest {
    connection_id: u64,
    command: SandboxControlCommand,
}

pub(crate) enum DispatchAdmission {
    Accepted,
    Busy,
    Closed,
}

pub(crate) struct Dispatcher {
    requests: SyncSender<DispatchRequest>,
    results: Receiver<DispatchResult>,
    wake_reader: UnixStream,
    ready: Arc<AtomicBool>,
    worker: JoinHandle<()>,
}

impl Dispatcher {
    pub(crate) fn start<S>(
        mut service: S,
        worker_thread_stack_bytes: usize,
    ) -> Result<Self, SandboxControlUdsError>
    where
        S: SandboxControlPort,
    {
        let (requests, request_receiver) = sync_channel::<DispatchRequest>(1);
        let (result_sender, results) = channel::<DispatchResult>();
        let (startup_sender, startup_receiver) = sync_channel::<()>(0);
        let ready = Arc::new(AtomicBool::new(false));
        let worker_ready = Arc::clone(&ready);
        let (wake_reader, mut wake_writer) = UnixStream::pair()
            .map_err(|error| io_error(SandboxControlUdsStage::Configure, error))?;
        wake_reader
            .set_nonblocking(true)
            .map_err(|error| io_error(SandboxControlUdsStage::Configure, error))?;
        wake_writer
            .set_nonblocking(true)
            .map_err(|error| io_error(SandboxControlUdsStage::Configure, error))?;
        let worker = thread::Builder::new()
            .name("actrail-sb-control-dispatch".to_string())
            .stack_size(worker_thread_stack_bytes)
            .spawn(move || {
                worker_ready.store(true, Ordering::Release);
                if startup_sender.send(()).is_err() {
                    return;
                }
                loop {
                    let Ok(request) = request_receiver.recv() else {
                        return;
                    };
                    let response = service.execute(request.command);
                    if result_sender
                        .send(DispatchResult {
                            connection_id: request.connection_id,
                            response,
                        })
                        .is_err()
                    {
                        return;
                    }
                    worker_ready.store(true, Ordering::Release);
                    match wake_writer.write(&[1]) {
                        Ok(_) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(_) => return,
                    }
                }
            })
            .map_err(|error| io_error(SandboxControlUdsStage::Configure, error))?;
        startup_receiver.recv().map_err(|_| {
            SandboxControlUdsError::new(
                SandboxControlUdsStage::Configure,
                "sandbox control dispatcher failed during startup",
            )
        })?;
        Ok(Self {
            requests,
            results,
            wake_reader,
            ready,
            worker,
        })
    }

    pub(crate) fn admit(
        &self,
        connection_id: u64,
        command: SandboxControlCommand,
    ) -> DispatchAdmission {
        if self
            .ready
            .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return DispatchAdmission::Busy;
        }
        match self.requests.try_send(DispatchRequest {
            connection_id,
            command,
        }) {
            Ok(()) => DispatchAdmission::Accepted,
            Err(TrySendError::Full(_)) => DispatchAdmission::Busy,
            Err(TrySendError::Disconnected(_)) => DispatchAdmission::Closed,
        }
    }

    pub(crate) fn wake_raw_fd(&self) -> RawFd {
        self.wake_reader.as_raw_fd()
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.worker.is_finished()
    }

    pub(crate) fn drain_results(&mut self) -> Vec<DispatchResult> {
        let mut wake = [0_u8; 64];
        loop {
            match self.wake_reader.read(&mut wake) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        let mut results = Vec::with_capacity(1);
        loop {
            match self.results.try_recv() {
                Ok(result) => results.push(result),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return results,
            }
        }
    }
}

fn io_error(stage: SandboxControlUdsStage, error: std::io::Error) -> SandboxControlUdsError {
    SandboxControlUdsError::new(stage, error.to_string())
}
