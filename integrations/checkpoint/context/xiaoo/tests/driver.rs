//! Drives the compiled integration through its ABI against a real Xiaoo socket.

use std::{
    io::{self, BufRead, Write},
    path::Path,
};

use abi_stable::library::lib_header_from_path;
use checkpoint::{
    BackendConfig, BackendError, BackendErrorKind, CheckpointBackend, CheckpointModuleRef,
    SnapshotContext, SnapshotId, TraceProcess,
};
use serde_json::{Value, json};

struct Driver {
    backend: CheckpointBackend,
}

impl Driver {
    fn execute(&mut self, command: &Value) -> Result<Value, BackendError> {
        match command["op"].as_str() {
            Some("connect") => {
                let trace_id = command["trace_id"]
                    .as_str()
                    .ok_or_else(|| Self::invalid("trace_id is required"))?;
                self.backend
                    .connect(&SnapshotContext {
                        trace_id: trace_id.into(),
                        trace_process: Self::trace_process()
                            .map_err(|error| Self::invalid(&error.to_string()))?,
                        scopes: Vec::new().into(),
                    })
                    .into_result()?;
            }
            Some("snapshot") => {
                let id = self.backend.snapshot().into_result()?;
                let value: Value = serde_json::from_slice(id.0.as_slice())
                    .map_err(|error| Self::invalid(&error.to_string()))?;
                return Ok(value);
            }
            Some("restore" | "delete") => {
                let bytes = serde_json::to_vec(&command["snapshot_id"])
                    .map_err(|error| Self::invalid(&error.to_string()))?;
                let id = SnapshotId(bytes.into());
                if command["op"] == "restore" {
                    self.backend
                        .restore(&id)
                        .into_result()
                        .map_err(|failure| failure.error)?;
                } else {
                    self.backend.delete(&id).into_result()?;
                }
            }
            Some("disconnect") => self.backend.disconnect().into_result()?,
            _ => return Err(Self::invalid("unknown driver operation")),
        }
        Ok(Value::Null)
    }

    fn trace_process() -> io::Result<TraceProcess> {
        let stat = std::fs::read_to_string("/proc/self/stat")?;
        let start_time_ticks = stat
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse()
            .map_err(io::Error::other)?;
        Ok(TraceProcess {
            host_pid: std::process::id(),
            start_time_ticks,
            pid_namespace: std::fs::read_link("/proc/self/ns/pid")?
                .to_string_lossy()
                .into_owned()
                .into(),
            mount_namespace: std::fs::read_link("/proc/self/ns/mnt")?
                .to_string_lossy()
                .into_owned()
                .into(),
            cwd: std::env::current_dir()?.to_string_lossy().into_owned().into(),
        })
    }

    fn invalid(message: &str) -> BackendError {
        BackendError {
            kind: BackendErrorKind::OperationFailed,
            message: message.into(),
        }
    }

    fn run(&mut self) -> io::Result<()> {
        for line in io::stdin().lock().lines() {
            let command: Value = serde_json::from_str(&line?).map_err(io::Error::other)?;
            let reply = match self.execute(&command) {
                Ok(result) => json!({"ok": true, "result": result}),
                Err(error) => {
                    json!({"ok": false, "kind": format!("{:?}", error.kind), "error": error.message.as_str()})
                }
            };
            println!("{reply}");
            io::stdout().flush()?;
        }
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        return Err(
            "usage: xiaoo_checkpoint_e2e <compiled-library> <integration-config.json>".into(),
        );
    }
    // The caller supplies the freshly built, trusted integration library.
    let header = lib_header_from_path(Path::new(&args[1]))?;
    let module = header.init_root_module::<CheckpointModuleRef>()?;
    let config = BackendConfig {
        json: std::fs::read_to_string(&args[2])?.into(),
    };
    let backend = (module.create())(config)
        .into_result()
        .map_err(|error| io::Error::other(error.message.to_string()))?;
    Driver { backend }.run()?;
    Ok(())
}
