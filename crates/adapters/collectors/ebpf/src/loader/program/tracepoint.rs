//! Collector-specific mapping for the shared tracepoint attach policy.

use libbpf_rs::{Link, ProgramMut};
use libbpf_tracepoint_attach::{
    TracepointAttachError, TracepointAttachOutcome, TracepointProgramAttacher,
    TracepointRequirement,
};

use crate::loader::LoaderError;
use crate::loader::environment;

pub(super) struct TracepointAttachPolicy {
    attacher: TracepointProgramAttacher,
}

impl TracepointAttachPolicy {
    pub(super) fn new() -> Self {
        Self {
            attacher: TracepointProgramAttacher::new(),
        }
    }

    pub(super) fn attach_program(
        &self,
        program: &ProgramMut<'_>,
        program_name: &str,
        allow_missing_tracepoint: bool,
    ) -> Result<Option<Link>, LoaderError> {
        let requirement = if allow_missing_tracepoint {
            TracepointRequirement::Optional
        } else {
            TracepointRequirement::Required
        };
        match self
            .attacher
            .attach(program, program_name, requirement)
            .map_err(map_attach_error)?
        {
            TracepointAttachOutcome::Attached(link) => Ok(Some(link)),
            TracepointAttachOutcome::Unavailable => Ok(None),
            TracepointAttachOutcome::NotTracepoint => program.attach().map(Some).map_err(|error| {
                LoaderError::new(
                    "attach_program",
                    format!(
                        "{error}; program={program_name}; {}",
                        environment::attach_environment_description()
                    ),
                )
            }),
        }
    }
}

fn map_attach_error(error: TracepointAttachError) -> LoaderError {
    LoaderError::new(
        error.stage(),
        format!(
            "{}; {}",
            error.detail(),
            environment::attach_environment_description()
        ),
    )
}
