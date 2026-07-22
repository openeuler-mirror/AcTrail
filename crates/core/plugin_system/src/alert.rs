//! Generic alert-write host boundary available to plugin callbacks.

use alert_contract::AlertDraft;
use model_core::ids::TraceId;
use model_core::trace::TraceAlertToken;

use crate::PluginRuntimeError;

pub trait AlertHost: Send + Sync {
    /// Admits an alert for asynchronous platform persistence.
    ///
    /// Success means the bounded platform ingress accepted the draft; it does
    /// not wait for SQLite commit and does not expose a storage identifier.
    fn submit_alert(
        &self,
        trace_id: TraceId,
        alert_token: TraceAlertToken,
        draft: AlertDraft,
    ) -> Result<(), PluginRuntimeError>;
}
