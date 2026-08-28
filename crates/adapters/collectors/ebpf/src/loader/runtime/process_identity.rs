//! Observer PID to raw kernel PID binding lifecycle.

use super::*;

const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;
const PROCESS_IDENTITY_RESOLVER_PROGRAM: &str = "resolve_process_identities";

pub(super) fn validate_process_identity_resolution_map(map: &MapHandle) -> Result<(), LoaderError> {
    if map.key_size() as usize != PROCESS_IDENTITY_RESOLUTION_KEY_SIZE
        || map.value_size() as usize != PROCESS_IDENTITY_RESOLUTION_VALUE_SIZE
    {
        return Err(LoaderError::new(
            "process_identity_resolution_map",
            format!(
                "unexpected resolution ABI key_size={} value_size={}",
                map.key_size(),
                map.value_size()
            ),
        ));
    }
    Ok(())
}

pub(super) fn configure_process_identity_resolution_ticks(
    object: &Object,
) -> Result<(), LoaderError> {
    let map = map_handle(
        object,
        "process_identity_resolution_tick_ns",
        "process_identity_resolution",
    )?;
    if map.key_size() as usize != std::mem::size_of::<u32>()
        || map.value_size() as usize != std::mem::size_of::<u64>()
        || map.max_entries() != 1
    {
        return Err(LoaderError::new(
            "process_identity_resolution",
            format!(
                "unexpected tick config ABI key_size={} value_size={} max_entries={}",
                map.key_size(),
                map.value_size(),
                map.max_entries()
            ),
        ));
    }
    let clock_ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let clock_ticks = u64::try_from(clock_ticks).map_err(|_| {
        LoaderError::new(
            "process_identity_resolution",
            format!("invalid sysconf(_SC_CLK_TCK) value {clock_ticks}"),
        )
    })?;
    if clock_ticks == 0 || NANOSECONDS_PER_SECOND % clock_ticks != 0 {
        return Err(LoaderError::new(
            "process_identity_resolution",
            format!(
                "kernel clock tick rate {clock_ticks} cannot represent exact process generations"
            ),
        ));
    }
    let tick_ns = NANOSECONDS_PER_SECOND / clock_ticks;
    map.update(&0_u32.to_ne_bytes(), &tick_ns.to_ne_bytes(), MapFlags::ANY)
        .map_err(|error| {
            LoaderError::new(
                "process_identity_resolution",
                format!("configure generation tick duration: {error}"),
            )
        })
}

impl EbpfRuntime {
    pub(crate) fn resolve_process_identities(
        &mut self,
        requests: &[ProcessIdentityResolutionRequest],
    ) -> Result<Vec<ResolvedProcessIdentity>, LoaderError> {
        if requests.is_empty() {
            return Ok(Vec::new());
        }
        let keys = requests
            .iter()
            .map(encode_process_identity_resolution_key)
            .collect::<Vec<_>>();
        let empty_value = [0_u8; PROCESS_IDENTITY_RESOLUTION_VALUE_SIZE];
        for key in &keys {
            if let Err(error) =
                self.process_identity_resolutions
                    .update(key, &empty_value, MapFlags::ANY)
            {
                let _ = self.clear_process_identity_resolution_keys(&keys);
                return Err(LoaderError::new(
                    "process_identity_resolution",
                    format!("publish resolution request: {error}"),
                ));
            }
        }

        let resolution_result = self.run_process_identity_resolver().and_then(|()| {
            requests
                .iter()
                .zip(&keys)
                .map(|(request, key)| self.read_process_identity_resolution(request, key))
                .collect::<Result<Vec<_>, _>>()
        });
        let cleanup_result = self.clear_process_identity_resolution_keys(&keys);
        match (resolution_result, cleanup_result) {
            (Ok(resolutions), Ok(())) => Ok(resolutions),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    fn run_process_identity_resolver(&mut self) -> Result<(), LoaderError> {
        let program = self
            .object
            .progs_mut()
            .find(|program| program.name() == OsStr::new(PROCESS_IDENTITY_RESOLVER_PROGRAM))
            .ok_or_else(|| {
                LoaderError::new(
                    "process_identity_resolution",
                    "task iterator program is missing",
                )
            })?;
        let link = program.attach().map_err(|error| {
            LoaderError::new(
                "process_identity_resolution",
                format!("attach task iterator: {error}"),
            )
        })?;
        let mut iterator = libbpf_rs::Iter::new(&link).map_err(|error| {
            LoaderError::new(
                "process_identity_resolution",
                format!("create task iterator: {error}"),
            )
        })?;
        let mut output = Vec::new();
        iterator.read_to_end(&mut output).map_err(|error| {
            LoaderError::new(
                "process_identity_resolution",
                format!("run task iterator: {error}"),
            )
        })?;
        Ok(())
    }

    fn read_process_identity_resolution(
        &self,
        request: &ProcessIdentityResolutionRequest,
        key: &[u8; PROCESS_IDENTITY_RESOLUTION_KEY_SIZE],
    ) -> Result<ResolvedProcessIdentity, LoaderError> {
        let value = self
            .process_identity_resolutions
            .lookup(key, MapFlags::ANY)
            .map_err(|error| {
                LoaderError::new(
                    "process_identity_resolution",
                    format!("read resolution result: {error}"),
                )
            })?
            .ok_or_else(|| {
                LoaderError::new(
                    "process_identity_resolution",
                    "resolution request disappeared during task iteration",
                )
            })?;
        let start_boottime_ns = u64::from_ne_bytes(value[0..8].try_into().map_err(|_| {
            LoaderError::new("process_identity_resolution", "truncated generation result")
        })?);
        let kernel_tgid = u32::from_ne_bytes(value[8..12].try_into().map_err(|_| {
            LoaderError::new(
                "process_identity_resolution",
                "truncated kernel TGID result",
            )
        })?);
        if kernel_tgid == 0 || start_boottime_ns == 0 {
            return Err(LoaderError::new(
                "process_identity_resolution",
                format!(
                    "observer TGID {} generation {} was not found by the kernel task iterator",
                    request.observer_tgid, request.start_time_ticks
                ),
            ));
        }
        Ok(ResolvedProcessIdentity {
            kernel_tgid,
            start_boottime_ns,
        })
    }

    fn clear_process_identity_resolution_keys(
        &self,
        keys: &[[u8; PROCESS_IDENTITY_RESOLUTION_KEY_SIZE]],
    ) -> Result<(), LoaderError> {
        for key in keys {
            if let Err(error) = self.process_identity_resolutions.delete(key)
                && error.kind() != libbpf_rs::ErrorKind::NotFound
            {
                return Err(LoaderError::new(
                    "process_identity_resolution",
                    format!("clear resolution request: {error}"),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn fork_trace_binding(
        &self,
        observer_pid: u32,
    ) -> Result<Option<(u32, ForkTraceBinding)>, LoaderError> {
        let key = observer_pid.to_ne_bytes();
        self.observer_fork_trace_bindings
            .lookup(&key, MapFlags::ANY)
            .map_err(|error| LoaderError::new("fork_trace_binding", error.to_string()))?
            .map(|value| parse_observer_fork_trace_binding(&value))
            .transpose()
    }

    pub(crate) fn fork_identity_publish_failures(&self) -> Result<u64, LoaderError> {
        read_event_transport_counter(
            &self.event_transport_diagnostics,
            FORK_IDENTITY_PUBLISH_FAIL_COUNTER,
        )
    }

    pub(crate) fn untrack_fork_host_pid(&self, observer_pid: u32) -> Result<(), LoaderError> {
        let observer_key = observer_pid.to_ne_bytes();
        let binding = self
            .observer_fork_trace_bindings
            .lookup(&observer_key, MapFlags::ANY)
            .map_err(|error| LoaderError::new("fork_trace_binding", error.to_string()))?
            .map(|value| parse_observer_fork_trace_binding(&value))
            .transpose()?;
        if binding.is_some() {
            self.observer_fork_trace_bindings
                .delete(&observer_key)
                .map_err(|error| LoaderError::new("fork_trace_binding", error.to_string()))?;
        }
        let Some((kernel_tgid, _)) = binding else {
            return Ok(());
        };
        let map_key = kernel_tgid.to_ne_bytes();
        if self
            .fork_trace_bindings
            .lookup(&map_key, MapFlags::ANY)
            .map_err(|error| LoaderError::new("fork_trace_binding", error.to_string()))?
            .is_some()
        {
            self.fork_trace_bindings
                .delete(&map_key)
                .map_err(|error| LoaderError::new("fork_trace_binding", error.to_string()))?;
        }
        Ok(())
    }

    pub(crate) fn untrack_fork_trace(&self, trace_id: TraceId) -> Result<(), LoaderError> {
        for key in self.fork_trace_bindings.keys().collect::<Vec<_>>() {
            let binding = self
                .fork_trace_bindings
                .lookup(&key, MapFlags::ANY)
                .map_err(|error| LoaderError::new("fork_trace_binding", error.to_string()))?
                .map(|value| parse_fork_trace_binding(&value))
                .transpose()?;
            if binding.is_some_and(|binding| binding.trace_id == trace_id) {
                self.fork_trace_bindings
                    .delete(&key)
                    .map_err(|error| LoaderError::new("fork_trace_binding", error.to_string()))?;
            }
        }
        for key in self.observer_fork_trace_bindings.keys().collect::<Vec<_>>() {
            let binding = self
                .observer_fork_trace_bindings
                .lookup(&key, MapFlags::ANY)
                .map_err(|error| LoaderError::new("fork_trace_binding", error.to_string()))?
                .map(|value| parse_observer_fork_trace_binding(&value))
                .transpose()?;
            if binding.is_some_and(|(_, binding)| binding.trace_id == trace_id) {
                self.observer_fork_trace_bindings
                    .delete(&key)
                    .map_err(|error| LoaderError::new("fork_trace_binding", error.to_string()))?;
            }
        }
        Ok(())
    }
}

fn encode_process_identity_resolution_key(
    request: &ProcessIdentityResolutionRequest,
) -> [u8; PROCESS_IDENTITY_RESOLUTION_KEY_SIZE] {
    let mut key = [0_u8; PROCESS_IDENTITY_RESOLUTION_KEY_SIZE];
    key[0..8].copy_from_slice(&request.start_time_ticks.to_ne_bytes());
    key[8..12].copy_from_slice(&request.observer_tgid.to_ne_bytes());
    key
}
