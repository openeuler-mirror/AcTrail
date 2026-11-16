//! Binds one trace to a Xiaoo session and forwards checkpoint operations.

use abi_stable::std_types::{RResult, RVec};
use checkpoint::{
    BackendError, RestorableCheckpointBackend, RestoreFailure, SnapshotContext, SnapshotId,
};

use crate::{
    client::Client,
    config::Config,
    error::Error,
    protocol::{Accepted, Context, InitConfig, Request, SaveMode, Session, Snapshot},
};

struct Binding {
    context: SnapshotContext,
    session_id: String,
}

pub(super) struct XiaooBackend {
    client: Client,
    binding: Option<Binding>,
}

impl XiaooBackend {
    pub fn new(config: Config) -> Self {
        Self {
            client: Client::new(config),
            binding: None,
        }
    }

    fn connect_session(&mut self, context: &SnapshotContext) -> Result<(), Error> {
        if let Some(binding) = &self.binding {
            return if binding.context.trace_id == context.trace_id {
                Ok(())
            } else {
                Err(Error::Operation(
                    "disconnect before binding another trace".into(),
                ))
            };
        }
        // Select the latest complete context boundary for this connection.
        let session: Session = self.client.request(&Request::Init {
            config: InitConfig {
                save_mode: SaveMode::Current,
            },
        })?;
        self.binding = Some(Binding {
            context: context.clone(),
            session_id: session.session_id,
        });
        Ok(())
    }

    fn context(&self) -> Result<Context<'_>, Error> {
        self.binding
            .as_ref()
            .map(|binding| Context {
                session_id: &binding.session_id,
            })
            .ok_or_else(|| Error::Operation("connect before using the Xiaoo backend".into()))
    }

    fn save(&self) -> Result<SnapshotId, Error> {
        let context = self.context()?;
        let snapshot: Snapshot = self.client.request(&Request::Save { context })?;
        self.check_session(&snapshot)?;
        let bytes =
            serde_json::to_vec(&snapshot).map_err(|error| Error::Operation(error.to_string()))?;
        Ok(SnapshotId(bytes.into()))
    }

    fn read_snapshot(&self, id: &SnapshotId) -> Result<Snapshot, Error> {
        let snapshot: Snapshot = serde_json::from_slice(id.0.as_slice())
            .map_err(|error| Error::Snapshot(error.to_string()))?;
        self.check_session(&snapshot)?;
        Ok(snapshot)
    }

    fn check_session(&self, snapshot: &Snapshot) -> Result<(), Error> {
        if snapshot.session_id != self.context()?.session_id {
            return Err(Error::Snapshot(
                "snapshot belongs to another Xiaoo session".into(),
            ));
        }
        Ok(())
    }

    fn restore_snapshot(&self, id: &SnapshotId) -> Result<(), Error> {
        let snapshot = self.read_snapshot(id)?;
        self.accept(&Request::Restore {
            context: self.context()?,
            snapshot_id: &snapshot.id,
            mode: self.client.restore_mode(),
        })
    }

    fn delete_snapshot(&self, id: &SnapshotId) -> Result<(), Error> {
        let snapshot = self.read_snapshot(id)?;
        self.accept(&Request::DeleteSnapshot {
            context: self.context()?,
            snapshot_id: &snapshot.id,
        })
    }

    fn accept(&self, request: &Request<'_>) -> Result<(), Error> {
        let response: Accepted = self.client.request(request)?;
        if response.accepted {
            Ok(())
        } else {
            Err(Error::Operation(
                "Xiaoo did not accept the operation".into(),
            ))
        }
    }

    fn release_session(&mut self) -> Result<(), Error> {
        if self.binding.is_some() {
            self.client.request::<()>(&Request::Release {
                context: self.context()?,
            })?;
            self.binding = None;
        }
        Ok(())
    }
}

impl RestorableCheckpointBackend for XiaooBackend {
    fn connect(&mut self, context: &SnapshotContext) -> RResult<(), BackendError> {
        self.connect_session(context)
            .map_err(BackendError::from)
            .into()
    }

    fn snapshot(&mut self) -> RResult<SnapshotId, BackendError> {
        self.save().map_err(BackendError::from).into()
    }

    fn restore(&mut self, snapshot_id: &SnapshotId) -> RResult<RVec<SnapshotId>, RestoreFailure> {
        self.restore_snapshot(snapshot_id)
            .map(|()| RVec::new())
            .map_err(|error| RestoreFailure::from(BackendError::from(error)))
            .into()
    }

    fn delete(&mut self, snapshot_id: &SnapshotId) -> RResult<(), BackendError> {
        self.delete_snapshot(snapshot_id)
            .map_err(BackendError::from)
            .into()
    }

    fn disconnect(&mut self) -> RResult<(), BackendError> {
        self.release_session().map_err(BackendError::from).into()
    }
}
