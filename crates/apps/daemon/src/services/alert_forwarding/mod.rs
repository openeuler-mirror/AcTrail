//! Daemon-owned lifecycle for the alert proxy connection.

mod link;
mod service;

pub(crate) use service::{
    ALERT_FORWARDING_INSTANCE_ID, ALERT_FORWARDING_PLUGIN_ID, AlertForwardingService,
};
