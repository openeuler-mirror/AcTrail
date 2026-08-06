use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::thread;
use std::time::Duration;

use alert_contract::AlertDraft;
use config_core::daemon::AlertWebhookConfig;
use model_core::ids::TraceId;

pub(super) struct WebhookDelivery {
    config: AlertWebhookConfig,
    agent: ureq::Agent,
}

impl WebhookDelivery {
    pub(super) fn new(config: AlertWebhookConfig) -> Self {
        let timeout = Duration::from_millis(config.timeout_ms);
        let agent_config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .build();
        let agent = ureq::Agent::new_with_config(agent_config);
        Self { config, agent }
    }

    pub(super) fn deliver(
        &self,
        trace_id: TraceId,
        plugin_id: &str,
        draft: &AlertDraft,
    ) -> Result<(), WebhookError> {
        let payload: serde_json::Value =
            serde_json::from_str(&draft.payload_json).unwrap_or(serde_json::Value::Null);

        let payload = if self.config.redact_sensitive_body {
            Self::redact_payload(payload)
        } else {
            payload
        };

        let body = serde_json::json!({
            "trace_id": trace_id.to_string(),
            "plugin_id": plugin_id,
            "definition_key": draft.definition_key,
            "payload": payload,
        });

        let mut request = self.agent.post(&self.config.url);
        if !self.config.auth_token.is_empty() {
            request = request.header("Authorization", &self.config.auth_token);
        }
        request = request.header("Content-Type", "application/json");

        let response = request.send_json(body).map_err(|error| {
            WebhookError::DeliveryFailed(format!("HTTP request failed: {error}"))
        })?;

        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(WebhookError::DeliveryFailed(format!(
                "webhook returned non-2xx status {status}"
            )));
        }
        Ok(())
    }

    fn redact_payload(value: serde_json::Value) -> serde_json::Value {
        const SENSITIVE_KEYS: &[&str] = &[
            "prompt",
            "response_body",
            "request_body",
            "api_key",
            "authorization",
            "api-key",
            "Authorization",
        ];
        match value {
            serde_json::Value::Object(map) => {
                let mut redacted = serde_json::Map::new();
                for (k, v) in map {
                    if SENSITIVE_KEYS.contains(&k.as_str()) {
                        redacted.insert(k, serde_json::Value::String("[redacted]".to_string()));
                    } else {
                        redacted.insert(k, Self::redact_payload(v));
                    }
                }
                serde_json::Value::Object(redacted)
            }
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.into_iter().map(Self::redact_payload).collect())
            }
            _ => value,
        }
    }
}

#[derive(Debug)]
pub(super) enum WebhookError {
    DeliveryFailed(String),
}

impl std::fmt::Display for WebhookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeliveryFailed(msg) => write!(f, "{msg}"),
        }
    }
}

pub(super) struct WebhookTask {
    pub trace_id: TraceId,
    pub plugin_id: String,
    pub instance_id: String,
    pub draft: AlertDraft,
}

pub(super) struct WebhookWorker {
    task_sender: Option<SyncSender<WebhookTask>>,
    error_receiver: Receiver<(TraceId, String, String)>,
    handle: Option<thread::JoinHandle<()>>,
}

impl WebhookWorker {
    pub(super) fn spawn(config: AlertWebhookConfig, queue_capacity: usize) -> Self {
        let (task_sender, task_receiver): (SyncSender<WebhookTask>, Receiver<WebhookTask>) =
            sync_channel(queue_capacity);
        let (error_sender, error_receiver): (
            SyncSender<(TraceId, String, String)>,
            Receiver<(TraceId, String, String)>,
        ) = sync_channel(queue_capacity);

        let handle = thread::Builder::new()
            .name("webhook-worker".into())
            .spawn(move || {
                let delivery = WebhookDelivery::new(config);
                while let Ok(task) = task_receiver.recv() {
                    if let Err(error) =
                        delivery.deliver(task.trace_id, &task.plugin_id, &task.draft)
                    {
                        let _ =
                            error_sender.send((task.trace_id, task.instance_id, error.to_string()));
                    }
                }
            })
            .expect("spawn webhook worker thread");

        Self {
            task_sender: Some(task_sender),
            error_receiver,
            handle: Some(handle),
        }
    }

    pub(super) fn enqueue(&self, task: WebhookTask) -> Result<(), WebhookTask> {
        match &self.task_sender {
            Some(sender) => sender.try_send(task).map_err(|e| match e {
                TrySendError::Full(t) | TrySendError::Disconnected(t) => t,
            }),
            None => Err(task),
        }
    }

    pub(super) fn drain_errors(&self) -> Vec<(TraceId, String, String)> {
        let mut errors = Vec::new();
        loop {
            match self.error_receiver.try_recv() {
                Ok(error) => errors.push(error),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        errors
    }
}

impl Drop for WebhookWorker {
    fn drop(&mut self) {
        drop(self.task_sender.take());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
