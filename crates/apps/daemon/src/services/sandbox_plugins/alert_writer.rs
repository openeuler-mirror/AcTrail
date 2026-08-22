use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use sandbox_resource_alert::{
    SandboxAlert, SandboxAlertKind, SandboxAlertSink, SandboxAlertSinkError,
};

pub(super) struct SandboxAlertWriter {
    client: Arc<SandboxAlertWriterClient>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl SandboxAlertWriter {
    pub(super) fn start(
        output_path: &Path,
        queue_capacity: usize,
        flush_interval: Duration,
        thread_stack_bytes: usize,
    ) -> io::Result<Self> {
        if !output_path.is_absolute()
            || queue_capacity == 0
            || flush_interval.is_zero()
            || thread_stack_bytes == 0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "sandbox alert output path must be absolute and capacities must be positive",
            ));
        }
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(output_path)?;
        let (sender, receiver) = mpsc::sync_channel(queue_capacity);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("actrail-sandbox-alert-writer".to_string())
            .stack_size(thread_stack_bytes)
            .spawn(move || AlertFileOwner::new(file, receiver, thread_stop).run(flush_interval))?;
        Ok(Self {
            client: Arc::new(SandboxAlertWriterClient { sender }),
            stop,
            thread: Some(thread),
        })
    }

    pub(super) fn sink(&self) -> Arc<dyn SandboxAlertSink> {
        self.client.clone()
    }

    pub(super) fn shutdown(&mut self) -> io::Result<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| io::Error::other("sandbox alert writer panicked"))??;
        }
        Ok(())
    }
}

impl Drop for SandboxAlertWriter {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

struct SandboxAlertWriterClient {
    sender: SyncSender<SandboxAlert>,
}

impl SandboxAlertSink for SandboxAlertWriterClient {
    fn try_submit(&self, alert: SandboxAlert) -> Result<(), SandboxAlertSinkError> {
        match self.sender.try_send(alert) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(SandboxAlertSinkError::new(
                "full",
                "sandbox alert writer queue is full",
            )),
            Err(TrySendError::Disconnected(_)) => Err(SandboxAlertSinkError::new(
                "closed",
                "sandbox alert writer is closed",
            )),
        }
    }
}

struct AlertFileOwner {
    writer: BufWriter<File>,
    receiver: Receiver<SandboxAlert>,
    stop: Arc<AtomicBool>,
}

impl AlertFileOwner {
    fn new(file: File, receiver: Receiver<SandboxAlert>, stop: Arc<AtomicBool>) -> Self {
        Self {
            writer: BufWriter::new(file),
            receiver,
            stop,
        }
    }

    fn run(mut self, flush_interval: Duration) -> io::Result<()> {
        loop {
            match self.receiver.recv_timeout(flush_interval) {
                Ok(alert) => self.write_alert(alert)?,
                Err(mpsc::RecvTimeoutError::Timeout) => self.writer.flush()?,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if self.stop.load(Ordering::Acquire) {
                while let Ok(alert) = self.receiver.try_recv() {
                    self.write_alert(alert)?;
                }
                break;
            }
        }
        self.writer.flush()
    }

    fn write_alert(&mut self, alert: SandboxAlert) -> io::Result<()> {
        write!(
            self.writer,
            "SANDBOX_ALERT gateway_id={} sb_id={} sequence={} ",
            alert.source.gateway_id(),
            alert.source.sb_id(),
            alert.batch_sequence
        )?;
        match alert.kind {
            SandboxAlertKind::OomKilled {
                sampled_at_ms,
                previous_count,
                current_count,
                delta,
                ..
            } => writeln!(
                self.writer,
                "kind=oom-killed sampled_at_ms={sampled_at_ms} previous={previous_count} current={current_count} delta={delta}"
            ),
            SandboxAlertKind::OomRisk {
                sampled_at_ms,
                available_bytes,
                threshold_bytes,
                ..
            } => writeln!(
                self.writer,
                "kind=oom-risk sampled_at_ms={sampled_at_ms} available_bytes={available_bytes} threshold_bytes={threshold_bytes}"
            ),
            SandboxAlertKind::HighRead {
                process,
                sample_started_ms,
                sample_ended_ms,
                bytes,
                threshold_bytes,
                ..
            } => writeln!(
                self.writer,
                "kind=high-read pid={} start_ticks={} sample_start_ms={} sample_end_ms={} bytes={} threshold_bytes={}",
                process.pid,
                process.start_time_ticks,
                sample_started_ms,
                sample_ended_ms,
                bytes,
                threshold_bytes
            ),
            SandboxAlertKind::HighWrite {
                process,
                sample_started_ms,
                sample_ended_ms,
                bytes,
                threshold_bytes,
                ..
            } => writeln!(
                self.writer,
                "kind=high-write pid={} start_ticks={} sample_start_ms={} sample_end_ms={} bytes={} threshold_bytes={}",
                process.pid,
                process.start_time_ticks,
                sample_started_ms,
                sample_ended_ms,
                bytes,
                threshold_bytes
            ),
        }
    }
}
