use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::broadcaster::AlertBroadcaster;
use crate::daemon_ingress::DaemonIngressServer;
use crate::diagnostics::ProxyDiagnostics;
use crate::registry::SubscriberRegistry;
use crate::subscriber::SubscriberServer;

use super::AlertProxyConfig;

pub struct AlertProxyBootstrap;

pub struct AlertProxyRuntime {
    ready: Arc<AtomicBool>,
    daemon_ingress: DaemonIngressServer,
    broadcaster: Arc<AlertBroadcaster>,
    subscriber: SubscriberServer,
}

impl AlertProxyBootstrap {
    pub fn start(config: AlertProxyConfig) -> Result<AlertProxyRuntime, String> {
        let ready = Arc::new(AtomicBool::new(false));
        let registry = Arc::new(SubscriberRegistry::new());
        let broadcaster = Arc::new(AlertBroadcaster::start(
            Arc::clone(&registry),
            config.subscriber.max_json_payload_bytes(),
            config.subscriber.broadcast_queue_capacity,
            config.subscriber.io_poll_interval(),
            config.subscriber.broadcaster_thread_stack_bytes,
        )?);
        let mut daemon_ingress = DaemonIngressServer::start(
            config.daemon_ingress.clone(),
            Arc::clone(&broadcaster),
            Arc::clone(&ready),
        )?;
        let subscriber = match SubscriberServer::start(config.subscriber, registry) {
            Ok(server) => server,
            Err(error) => {
                let _ = daemon_ingress.shutdown();
                let _ = broadcaster.shutdown();
                return Err(error);
            }
        };
        ready.store(true, Ordering::Release);
        ProxyDiagnostics::ready(&config.daemon_ingress.socket_path, subscriber.local_addr());
        Ok(AlertProxyRuntime {
            ready,
            daemon_ingress,
            broadcaster,
            subscriber,
        })
    }
}

impl AlertProxyRuntime {
    pub fn shutdown(&mut self) -> Result<(), String> {
        self.ready.store(false, Ordering::Release);
        let daemon = self.daemon_ingress.shutdown();
        let broadcaster = self.broadcaster.shutdown();
        let subscriber = self.subscriber.shutdown();
        combine_shutdown(daemon, broadcaster, subscriber)
    }
}

impl Drop for AlertProxyRuntime {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn combine_shutdown(
    daemon: Result<(), String>,
    broadcaster: Result<(), String>,
    subscriber: Result<(), String>,
) -> Result<(), String> {
    let mut errors = Vec::new();
    if let Err(error) = daemon {
        errors.push(format!("daemon ingress shutdown: {error}"));
    }
    if let Err(error) = broadcaster {
        errors.push(format!("broadcaster shutdown: {error}"));
    }
    if let Err(error) = subscriber {
        errors.push(format!("subscriber shutdown: {error}"));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}
