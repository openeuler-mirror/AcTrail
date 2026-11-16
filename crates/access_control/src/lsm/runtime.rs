//! Load governance hooks with shared scope enrollment and configured decisions.

use super::{audit::LsmAuditEvent, controller::LsmController, publisher::PolicyPublisher};
use libbpf_rs::{
    Link, MapCore, MapFlags, MapHandle, Object, ObjectBuilder, RingBuffer, RingBufferBuilder,
};
use std::os::fd::AsFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Co-owned by the observation runtime whose process membership maps are reused.
pub struct LsmRuntime {
    ring: RingBuffer<'static>,
    _links: Vec<Link>,
    _publisher: Arc<PolicyPublisher>,
    _object: Object,
    // Links and object are released before the exclusive device session.
    _grey: Option<super::grey::LsmGreyBroker>,
    pending: Arc<Mutex<Vec<LsmAuditEvent>>>,
    batch_max: usize,
    counters: MapHandle,
    last_audit_dropped: AtomicU64,
}

impl LsmRuntime {
    pub fn attach(controller: &Arc<LsmController>, scope: &Object) -> Result<Self, String> {
        let mode = controller.config.grey.mode;
        let (object, grey) = if mode == config_core::daemon::LsmGreyMode::Disabled {
            (Self::load_object(controller, scope, false)?, None)
        } else {
            let prepare = || {
                let broker = super::grey::LsmGreyBroker::start(
                    &controller.config.grey,
                    controller.grey_handler.clone(),
                )?;
                let object = Self::load_object(controller, scope, true)?;
                Ok::<_, String>((object, Some(broker)))
            };
            match prepare() {
                Ok(session) => {
                    tracing::info!("LSM grey dynamic decisions enabled");
                    session
                }
                Err(error) if mode == config_core::daemon::LsmGreyMode::Auto => {
                    tracing::warn!(%error, "LSM grey unavailable; using ordinary LSM with rule fallback");
                    (Self::load_object(controller, scope, false)?, None)
                }
                Err(error) => return Err(error),
            }
        };
        let slots = object
            .maps()
            .find(|map| map.name() == "ac_policy_slots")
            .ok_or("LSM policy map missing")?;
        let publisher = Arc::new(PolicyPublisher::new(
            MapHandle::try_from(&slots).map_err(|error| error.to_string())?,
            controller.config.policy_max_entries,
        ));
        controller.bind(&publisher)?;
        let pending = Arc::new(Mutex::new(Vec::new()));
        let receiver = pending.clone();
        let batch_max = controller.config.audit_batch_max as usize;
        let mut ring_builder = RingBufferBuilder::new();
        let events = object
            .maps()
            .find(|map| map.name() == "ac_events")
            .ok_or("LSM audit map missing")?;
        ring_builder
            .add(&events, move |raw| {
                match LsmAuditEvent::decode(raw) {
                    Ok(event) => match receiver.lock() {
                        Ok(mut pending) => pending.push(event),
                        Err(_) => return -1,
                    },
                    Err(error) => tracing::error!(%error, "invalid LSM audit record"),
                }
                0
            })
            .map_err(|error| error.to_string())?;
        let ring = ring_builder
            .build()
            .map_err(|error| format!("LSM audit consumer: {error}"))?;
        let counters = object
            .maps()
            .find(|map| map.name() == "ac_counters")
            .ok_or("LSM counters missing")?;
        let counters = MapHandle::try_from(&counters).map_err(|error| error.to_string())?;
        let mut links = Vec::new();
        for program in object.progs_mut() {
            if !program.autoload() {
                continue;
            }
            links.push(program.attach_lsm().map_err(|error| {
                format!(
                    "attach BPF LSM {}: {error}",
                    program.name().to_string_lossy()
                )
            })?);
        }
        Ok(Self {
            ring,
            _links: links,
            _publisher: publisher,
            _object: object,
            _grey: grey,
            pending,
            batch_max,
            counters,
            last_audit_dropped: AtomicU64::new(0),
        })
    }

    // Failed grey objects drop before retrying the ordinary variant.
    // Policy publication stays outside this fallback, so policy errors remain fatal.
    fn load_object(
        controller: &Arc<LsmController>,
        scope: &Object,
        grey: bool,
    ) -> Result<Object, String> {
        let observer_binding = scope
            .maps()
            .any(|map| map.name() == "pending_exec_observer_bindings");
        let bytes: &[u8] = match (grey, observer_binding) {
            (false, true) => include_bytes!(env!("ACTRAIL_LSM_TASK_OBJECT")),
            (false, false) => include_bytes!(env!("ACTRAIL_LSM_HASH_OBJECT")),
            (true, true) => include_bytes!(env!("ACTRAIL_LSM_GREY_TASK_OBJECT")),
            (true, false) => include_bytes!(env!("ACTRAIL_LSM_GREY_HASH_OBJECT")),
        };
        let mut builder = ObjectBuilder::default();
        let mut object = builder
            .open_memory(bytes)
            .map_err(|error| format!("open BPF LSM: {error}"))?;
        for mut map in object.maps_mut() {
            let name = map.name().to_string_lossy().into_owned();
            match name.as_str() {
                "ac_policy_template" => map
                    .set_max_entries(controller.config.policy_max_entries)
                    .map_err(|error| error.to_string())?,
                "ac_events" => map
                    .set_max_entries(controller.config.audit_buffer_bytes)
                    .map_err(|error| error.to_string())?,
                "pending_exec_count"
                | "pending_exec_generation_tick_ns"
                | "observer_pid_namespace"
                | "pending_exec_observer_bindings"
                | "pending_exec_bindings"
                | "pending_exec_pid_index" => {
                    let source = scope
                        .maps()
                        .find(|source| source.name() == name.as_str())
                        .ok_or_else(|| format!("LSM scope map {name} is missing"))?;
                    map.reuse_fd(source.as_fd())
                        .map_err(|error| format!("reuse LSM scope map {name}: {error}"))?;
                }
                _ => (),
            }
        }
        for mut program in object.progs_mut() {
            let enabled = match program.name().to_str() {
                Some("ac_task_alloc" | "ac_bprm_scope") => true,
                Some("ac_bprm_check") => controller.command_enabled(),
                Some(
                    "ac_socket_connect" | "ac_socket_sendmsg" | "ac_socket_bind"
                    | "ac_socket_listen",
                ) => controller.network_config.enabled,
                Some(
                    "ac_file_open" | "ac_file_permission" | "ac_mmap_file" | "ac_file_mprotect"
                    | "ac_path_unlink" | "ac_path_chmod" | "ac_path_chown" | "ac_path_truncate"
                    | "ac_file_truncate" | "ac_path_rename" | "ac_path_link",
                ) => controller.file_domain_enabled(1),
                Some("ac_path_mkdir") => controller.file_domain_enabled(2),
                Some("ac_path_rmdir") => controller.file_domain_enabled(3),
                _ => false,
            };
            program.set_autoload(enabled);
        }
        let object = object
            .load()
            .map_err(|error| format!("load BPF LSM: {error}"))?;
        Ok(object)
    }

    pub fn poll_fd(&self) -> i32 {
        self.ring.epoll_fd()
    }

    pub fn drain(&self) -> Result<Vec<LsmAuditEvent>, String> {
        let result = self.ring.consume_raw_n(self.batch_max);
        if result < 0 {
            return Err(format!("consume LSM audit: errno {}", -result));
        }
        match self
            .counters
            .lookup_percpu(&0_u32.to_ne_bytes(), MapFlags::ANY)
        {
            Ok(Some(values)) => {
                let dropped: u64 = values
                    .iter()
                    .filter_map(|value| value.as_slice().try_into().ok())
                    .map(u64::from_ne_bytes)
                    .sum();
                let previous = self.last_audit_dropped.swap(dropped, Ordering::Relaxed);
                if dropped > previous {
                    tracing::warn!(
                        dropped = dropped - previous,
                        total = dropped,
                        "LSM audit ring full; access decisions continued in kernel"
                    );
                }
            }
            Err(error) => tracing::warn!(%error, "cannot read LSM audit loss counter"),
            Ok(None) => (),
        }
        Ok(std::mem::take(
            &mut *self.pending.lock().map_err(|_| "LSM audit lock poisoned")?,
        ))
    }
}
