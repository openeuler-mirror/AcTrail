use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use crate::{ExportDeliveryDrop, ExportError, ExportPublishResult};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BestEffortDeliveryConfig {
    pub component_name: &'static str,
    pub worker_thread_name: &'static str,
    pub queue_capacity: u32,
}

pub trait BestEffortSink<T>: Send + 'static {
    fn deliver(&mut self, message: T) -> Result<(), String>;

    fn finish(&mut self) -> Result<(), String> {
        Ok(())
    }
}

pub struct BestEffortDelivery<T> {
    sender: Option<SyncSender<T>>,
    worker: Option<JoinHandle<()>>,
    error: Arc<Mutex<Option<String>>>,
    component_name: &'static str,
    queue_capacity: u32,
}

impl<T: Send + 'static> BestEffortDelivery<T> {
    pub fn spawn<S>(
        config: BestEffortDeliveryConfig,
        sink: S,
    ) -> Result<BestEffortDelivery<T>, ExportError>
    where
        S: BestEffortSink<T>,
    {
        if config.queue_capacity == u32::default() {
            return Err(ExportError::new(
                config.component_name,
                "queue capacity must be positive",
            ));
        }
        let queue_capacity = usize::try_from(config.queue_capacity).map_err(|error| {
            ExportError::new(
                config.component_name,
                format!("queue capacity overflow: {error}"),
            )
        })?;
        let (sender, receiver) = sync_channel(queue_capacity);
        let error = Arc::new(Mutex::new(None));
        let thread_error = Arc::clone(&error);
        let worker = thread::Builder::new()
            .name(config.worker_thread_name.to_string())
            .spawn(move || {
                let mut sink = sink;
                while let Ok(message) = receiver.recv() {
                    if let Err(error) = sink.deliver(message) {
                        store_delivery_error(&thread_error, error);
                        return;
                    }
                }
                if let Err(error) = sink.finish() {
                    store_delivery_error(&thread_error, error);
                }
            })
            .map_err(|error| {
                ExportError::new(
                    config.component_name,
                    format!("spawn delivery worker failed: {error}"),
                )
            })?;

        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
            error,
            component_name: config.component_name,
            queue_capacity: config.queue_capacity,
        })
    }

    pub fn check_health(&self) -> Result<(), ExportError> {
        let error = self.error.lock().map_err(|error| {
            self.delivery_error(format!("delivery error lock poisoned: {error}"))
        })?;
        match error.as_ref() {
            Some(message) => Err(self.delivery_error(message.clone())),
            None => Ok(()),
        }
    }

    pub fn publish(&self, message: T) -> Result<ExportPublishResult, ExportError> {
        self.check_health()?;
        let Some(sender) = &self.sender else {
            return Err(self.delivery_error("delivery sender is closed"));
        };
        match sender.try_send(message) {
            Ok(()) => Ok(ExportPublishResult::delivered()),
            Err(TrySendError::Full(_)) => Ok(ExportPublishResult::dropped(
                ExportDeliveryDrop::queue_full(1, self.queue_capacity),
            )),
            Err(TrySendError::Disconnected(_)) => {
                self.check_health()?;
                Err(self.delivery_error("delivery worker disconnected"))
            }
        }
    }

    fn delivery_error(&self, message: impl Into<String>) -> ExportError {
        ExportError::new(self.component_name, message).with_queue_capacity(self.queue_capacity)
    }
}

impl<T> Drop for BestEffortDelivery<T> {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn store_delivery_error(error: &Arc<Mutex<Option<String>>>, message: String) {
    if let Ok(mut slot) = error.lock() {
        *slot = Some(message);
    }
}
