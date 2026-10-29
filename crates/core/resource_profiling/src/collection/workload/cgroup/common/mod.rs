//! Version-agnostic contracts and shared machinery for cgroup accounting.
//!
//! `managed/` owns the lifecycle of AcTrail-created trace scopes; each cgroup
//! version plugs in through `ManagedHierarchy`. `sampler/` turns one cgroup's
//! counters into a workload sample for managed scopes and bindings alike.

mod availability;
pub(crate) mod managed;
pub(crate) mod sampler;
pub(crate) use availability::{CgroupAvailability, cgroup_error};
