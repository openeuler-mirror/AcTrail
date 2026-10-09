//! Trace-scoped access enforcement services.

mod fanotify;
mod service;

pub(super) use service::{
    COLLECTOR_NAME, EnforcementDrain, FileEnforcementService, descriptor as enforcement_descriptor,
};
