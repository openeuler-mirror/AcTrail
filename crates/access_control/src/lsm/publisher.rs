use super::compiler::PolicySnapshot;
use libbpf_rs::{MapCore, MapFlags, MapHandle, MapType, libbpf_sys};
use std::os::fd::{AsFd, AsRawFd};

pub(super) struct PolicyPublisher {
    slots: MapHandle,
    capacity: u32,
}

impl PolicyPublisher {
    pub(super) fn new(slots: MapHandle, capacity: u32) -> Self {
        Self { slots, capacity }
    }

    pub(super) fn publish(&self, snapshot: &PolicySnapshot) -> Result<(), String> {
        let opts = libbpf_sys::bpf_map_create_opts {
            sz: std::mem::size_of::<libbpf_sys::bpf_map_create_opts>() as _,
            map_flags: libbpf_sys::BPF_F_NO_PREALLOC,
            ..Default::default()
        };
        let map = MapHandle::create(
            MapType::LpmTrie,
            Some("ac_policy"),
            260,
            288,
            self.capacity,
            &opts,
        )
        .map_err(|error| format!("create LSM policy snapshot: {error}"))?;
        for (key, value) in &snapshot.entries {
            map.update(key, value, MapFlags::ANY)
                .map_err(|error| format!("populate LSM policy snapshot: {error}"))?;
        }
        map.freeze()
            .map_err(|error| format!("freeze LSM policy snapshot: {error}"))?;
        // The outer map takes a kernel reference. No old inner map is mutated.
        self.slots
            .update(
                &0_u32.to_ne_bytes(),
                &map.as_fd().as_raw_fd().to_ne_bytes(),
                MapFlags::ANY,
            )
            .map_err(|error| format!("publish LSM policy snapshot: {error}"))
    }
}
