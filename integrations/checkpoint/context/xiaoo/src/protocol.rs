//! Declares Xiaoo's newline-delimited JSON requests and response payloads.

use serde::{Deserialize, Serialize};

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SaveMode {
    Current,
}

#[derive(Serialize)]
pub(super) struct InitConfig {
    pub save_mode: SaveMode,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RestoreMode {
    PromiseMode,
    EnsureSafeMode,
}

#[derive(Serialize)]
pub(super) struct Context<'a> {
    pub session_id: &'a str,
}

#[derive(Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(super) enum Request<'a> {
    Init {
        config: InitConfig,
    },
    Save {
        context: Context<'a>,
    },
    Restore {
        context: Context<'a>,
        snapshot_id: &'a str,
        mode: RestoreMode,
    },
    DeleteSnapshot {
        context: Context<'a>,
        snapshot_id: &'a str,
    },
    Release {
        context: Context<'a>,
    },
}

#[derive(Deserialize)]
pub(super) struct Response {
    pub ok: bool,
    #[serde(default)]
    pub result: serde_json::Value,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct Session {
    pub session_id: String,
}

#[derive(Deserialize, Serialize)]
pub(super) struct Snapshot {
    pub id: String,
    pub session_id: String,
}

#[derive(Deserialize)]
pub(super) struct Accepted {
    pub accepted: bool,
}
