//! Trace-scoped access enforcement services.

mod fanotify;
mod rules;
mod service;

pub(super) use service::{
    COLLECTOR_NAME, FanotifyEnforcementService, descriptor as enforcement_descriptor,
};
