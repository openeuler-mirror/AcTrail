use std::collections::{BTreeMap, BTreeSet};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex};

use alert_delivery_contract::DeliverySeverity;

pub(crate) struct SubscriberRegistry {
    sessions: Mutex<BTreeMap<String, Arc<SubscriberHandle>>>,
}

pub(crate) struct SubscriberHandle {
    session_id: String,
    delivery: Mutex<DeliveryState>,
    closer: TcpStream,
    closed: AtomicBool,
}

#[derive(Clone)]
pub(crate) struct Subscription {
    topics: BTreeSet<String>,
    severity_mask: u8,
}

struct DeliveryState {
    subscription: Option<Subscription>,
    outbound: SyncSender<Arc<[u8]>>,
}

impl SubscriberRegistry {
    pub(crate) fn new() -> Self {
        Self {
            sessions: Mutex::new(BTreeMap::new()),
        }
    }

    pub(crate) fn register(&self, session: Arc<SubscriberHandle>) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "subscriber registry lock is poisoned".to_string())?;
        if sessions
            .insert(session.session_id.clone(), session)
            .is_some()
        {
            return Err("subscriber session id collision".to_string());
        }
        Ok(())
    }

    pub(crate) fn remove(&self, session_id: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(session_id);
        }
    }

    pub(crate) fn snapshot(&self) -> Vec<Arc<SubscriberHandle>> {
        self.sessions
            .lock()
            .map(|sessions| sessions.values().cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) fn close_all(&self) {
        for session in self.snapshot() {
            session.close();
        }
    }
}

impl SubscriberHandle {
    pub(crate) fn new(
        session_id: String,
        outbound: SyncSender<Arc<[u8]>>,
        closer: TcpStream,
    ) -> Self {
        Self {
            session_id,
            delivery: Mutex::new(DeliveryState {
                subscription: None,
                outbound,
            }),
            closer,
            closed: AtomicBool::new(false),
        }
    }

    pub(crate) fn accept_subscription(
        &self,
        subscription: Subscription,
        confirmation: Arc<[u8]>,
    ) -> Result<(), String> {
        let mut delivery = self
            .delivery
            .lock()
            .map_err(|_| "subscriber delivery lock is poisoned".to_string())?;
        delivery
            .outbound
            .try_send(confirmation)
            .map_err(|_| "subscriber outbound queue rejected subscription response".to_string())?;
        delivery.subscription = Some(subscription);
        Ok(())
    }

    pub(crate) fn matching_sender(
        &self,
        category: &str,
        severity: DeliverySeverity,
    ) -> Option<SyncSender<Arc<[u8]>>> {
        if self.closed.load(Ordering::Acquire) {
            return None;
        }
        let delivery = self.delivery.lock().ok()?;
        delivery
            .subscription
            .as_ref()
            .filter(|subscription| {
                subscription.topics.contains(category) && subscription.accepts_severity(severity)
            })
            .map(|_| delivery.outbound.clone())
    }

    pub(crate) fn try_deliver(&self, outbound: &SyncSender<Arc<[u8]>>, frame: Arc<[u8]>) {
        match outbound.try_send(frame) {
            Ok(()) => {}
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => self.close(),
        }
    }

    pub(crate) fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            let _ = self.closer.shutdown(Shutdown::Both);
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

impl Subscription {
    pub(crate) fn new(topics: Vec<String>, severities: Vec<DeliverySeverity>) -> Self {
        let severity_mask = severities
            .into_iter()
            .fold(0_u8, |mask, severity| mask | Self::severity_bit(severity));
        Self {
            topics: topics.into_iter().collect(),
            severity_mask,
        }
    }

    pub(crate) fn validates_severities(severities: &[DeliverySeverity]) -> bool {
        if severities.len() > 3 {
            return false;
        }
        let mut mask = 0_u8;
        for severity in severities {
            let bit = Self::severity_bit(*severity);
            if mask & bit != 0 {
                return false;
            }
            mask |= bit;
        }
        true
    }

    fn accepts_severity(&self, severity: DeliverySeverity) -> bool {
        self.severity_mask == 0 || self.severity_mask & Self::severity_bit(severity) != 0
    }

    const fn severity_bit(severity: DeliverySeverity) -> u8 {
        1_u8 << severity.code()
    }
}
