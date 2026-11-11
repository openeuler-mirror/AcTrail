//! Checkpoint glue of the service host: delayed `TrackAdd` replies, command authorization and
//! the end of checkpoint Traces.

use std::path::Path;
use std::time::SystemTime;

use config_core::daemon::{CheckpointBackendConfig, CheckpointRuntimeConfig};
use control_contract::checkpoint::{CheckpointAction, LaunchCheckpointConfig};
use control_contract::reply::{ControlError, TrackAddReply};
use model_core::ids::TraceId;
use uds_control_server::{ControlOutcome, ReplyTicket};

use super::{AttachService, DaemonServiceHost};
use crate::peer_identity::PeerIdentity;
use crate::services::checkpoint::{CheckpointService, launch_request};

impl<A> DaemonServiceHost<A> {
    pub(super) fn checkpoint_service(&mut self) -> Result<&mut CheckpointService, ControlError> {
        self.checkpoint.as_mut().ok_or_else(checkpoint_disabled)
    }
}

impl<A> DaemonServiceHost<A>
where
    A: AttachService,
{
    /// Opens the checkpoint catalog and loads the integrations.
    pub fn enable_checkpoints(
        &mut self,
        catalog_path: &Path,
        backends: &[CheckpointBackendConfig],
        runtime: CheckpointRuntimeConfig,
    ) -> Result<(), ControlError> {
        let (checkpoint, trace_id_seed) = CheckpointService::open(catalog_path, backends, runtime)?;
        self.wiring
            .trace_runtime
            .advance_trace_id_seed(trace_id_seed);
        self.checkpoint = Some(checkpoint);
        Ok(())
    }

    /// The attached Trace stays stopped at the launch barrier until its initial checkpoint
    /// exists; `TrackAdd` is answered then. If the checkpoint cannot start, the Trace is removed.
    pub(super) fn start_checkpoint_launch(
        &mut self,
        added: TrackAddReply,
        config: LaunchCheckpointConfig,
    ) -> ControlOutcome {
        let trace_id = added.trace_id;
        match self.begin_checkpoint_launch(added, config) {
            Ok(ticket) => ControlOutcome::Later(ticket),
            Err(error) => {
                self.remove_checkpoint_trace(trace_id);
                ControlOutcome::Reply(Err(error))
            }
        }
    }

    fn begin_checkpoint_launch(
        &mut self,
        added: TrackAddReply,
        config: LaunchCheckpointConfig,
    ) -> Result<ReplyTicket, ControlError> {
        let entry = self
            .wiring
            .trace_runtime
            .get_trace(added.trace_id)
            .ok_or_else(|| ControlError::new("checkpoint_launch", "the attached Trace is gone"))?;
        let owner = entry
            .owner
            .clone()
            .ok_or_else(|| ControlError::new("checkpoint_launch", "the Trace has no owner"))?;
        let root = entry.trace.root_process_identity;
        let root_pid = self.wiring.attach_service.host_pid_for_process(root)?;
        let request = launch_request(added.trace_id, &owner, root_pid, config)?;
        self.checkpoint_service()?.start_launch(request, added)
    }

    /// Checkpoint commands are authorized against the owner recorded at launch, so they keep
    /// working after the Trace leaves the runtime.
    pub(super) fn authorize_checkpoint(
        &self,
        peer: &PeerIdentity,
        action: &CheckpointAction,
    ) -> Result<(), ControlError> {
        if peer.is_trusted_host_root() {
            return Ok(());
        }
        let checkpoint = self.checkpoint.as_ref().ok_or_else(checkpoint_disabled)?;
        let trace_id = checkpoint.command_trace(action)?;
        let owner = checkpoint.trace_owner(trace_id)?;
        peer.authorize_trace_owner(trace_id, &owner)
    }

    /// Runs after live events: hands the collected LLM requests to checkpoint work, releases the
    /// responses it is done with and removes Traces whose initial checkpoint failed.
    pub(super) fn drain_checkpoint(&mut self) {
        let Some(checkpoint) = self.checkpoint.as_mut() else {
            return;
        };
        let attach = &mut self.wiring.attach_service;
        checkpoint.update_processes(&self.wiring.trace_runtime, &|identity| {
            attach.process_record(identity)
        });
        let llm_requests = attach.take_llm_requests();
        let drained = checkpoint.drain(&self.wiring.trace_runtime, llm_requests);
        for (trace_id, watch) in drained.watch {
            attach.watch_trace(trace_id, watch);
        }
        for hold in drained.release {
            attach.release_response(hold);
        }
        for trace_id in drained.failed_launches {
            self.remove_checkpoint_trace(trace_id);
        }
    }

    fn remove_checkpoint_trace(&mut self, trace_id: TraceId) {
        if let Err(error) = self.wiring.attach_service.remove_root(
            &mut self.wiring.trace_runtime,
            trace_id,
            SystemTime::now(),
        ) {
            tracing::warn!(
                %trace_id,
                code = %error.code,
                message = %error.message,
                "removing a Trace whose initial checkpoint failed"
            );
        }
    }
}

pub(super) fn checkpoint_disabled() -> ControlError {
    ControlError::new(
        "checkpoint_disabled",
        "checkpoints are not enabled; set checkpoint.enabled in the daemon configuration",
    )
}
