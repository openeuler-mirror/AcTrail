use std::ffi::OsString;

use super::{EnvironmentContext, InjectionResources, LaunchInjection, PreparationContext};

pub(super) struct NoopInjection;

impl LaunchInjection for NoopInjection {
    fn prepare(
        &mut self,
        _command: &mut Vec<OsString>,
        _context: PreparationContext<'_>,
    ) -> Result<InjectionResources, String> {
        Ok(InjectionResources::default())
    }

    fn envs(&self, _context: EnvironmentContext<'_>) -> Result<Vec<(OsString, OsString)>, String> {
        Ok(Vec::new())
    }
}
