use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use alert_delivery_contract::{ExternalAlert, ForwardAlert, JsonFrameCodec};
use uuid::Uuid;

use crate::registry::SubscriberRegistry;

pub(crate) struct AlertBroadcaster {
    sender: SyncSender<ForwardAlert>,
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

struct BroadcastWorker {
    registry: Arc<SubscriberRegistry>,
    json_codec: JsonFrameCodec,
    receiver: Receiver<ForwardAlert>,
    stop: Arc<AtomicBool>,
    poll_interval: Duration,
}

impl AlertBroadcaster {
    pub(crate) fn start(
        registry: Arc<SubscriberRegistry>,
        max_frame_bytes: usize,
        queue_capacity: usize,
        poll_interval: Duration,
        thread_stack_bytes: usize,
    ) -> Result<Self, String> {
        let json_codec = JsonFrameCodec::new(max_frame_bytes).map_err(|error| error.to_string())?;
        let (sender, receiver) = mpsc::sync_channel(queue_capacity);
        let stop = Arc::new(AtomicBool::new(false));
        let worker = BroadcastWorker {
            registry,
            json_codec,
            receiver,
            stop: Arc::clone(&stop),
            poll_interval,
        };
        let thread = thread::Builder::new()
            .name("alert-proxy-broadcaster".to_string())
            .stack_size(thread_stack_bytes)
            .spawn(move || worker.run())
            .map_err(|error| format!("spawn alert broadcaster: {error}"))?;
        Ok(Self {
            sender,
            stop,
            thread: Mutex::new(Some(thread)),
        })
    }

    pub(crate) fn try_publish(&self, alert: ForwardAlert) {
        match self.sender.try_send(alert) {
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
        }
    }

    pub(crate) fn shutdown(&self) -> Result<(), String> {
        self.stop.store(true, Ordering::Release);
        let thread = self
            .thread
            .lock()
            .map_err(|_| "alert broadcaster thread lock is poisoned".to_string())?
            .take();
        match thread {
            Some(thread) => thread
                .join()
                .map_err(|_| "alert broadcaster panicked".to_string()),
            None => Ok(()),
        }
    }
}

impl Drop for AlertBroadcaster {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

impl BroadcastWorker {
    fn run(self) {
        while !self.stop.load(Ordering::Acquire) {
            match self.receiver.recv_timeout(self.poll_interval) {
                Ok(alert) => self.publish(alert),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn publish(&self, alert: ForwardAlert) {
        let sessions = self.registry.snapshot();
        if sessions.is_empty() {
            return;
        }
        let sessions = sessions
            .into_iter()
            .filter_map(|session| {
                session
                    .matching_sender(&alert.category, alert.severity)
                    .map(|sender| (session, sender))
            })
            .collect::<Vec<_>>();
        if sessions.is_empty() {
            return;
        }
        let external = ExternalAlert {
            id: Uuid::new_v4().to_string(),
            ts: alert.detected_at_ms,
            source: alert.source.into(),
            s: alert.severity,
            cat: alert.category,
            description: alert.description,
            labels: serde_json::Map::new(),
            extras: alert.extras,
        };
        let Ok(frame) = self.json_codec.encode(&external) else {
            return;
        };
        let frame: Arc<[u8]> = frame.into();
        for (session, sender) in sessions {
            session.try_deliver(&sender, Arc::clone(&frame));
        }
    }
}
