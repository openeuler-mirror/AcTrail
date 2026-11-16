//! cgroup accounting: interfaces at this level, shared contracts in `common/`,
//! construction in `cgroup_backend_factory`, one directory per cgroup version
//! under `implementations/`.

mod cgroup_backend;
mod cgroup_backend_factory;
mod cgroup_persistence;
mod cgroup_resource_profiling_backend;
mod cgroup_sampling_backend;
mod common;
mod config;
mod implementations;
pub(crate) use cgroup_backend_factory::CgroupBackendFactory;
pub use cgroup_persistence::CgroupPersistence;
pub use cgroup_resource_profiling_backend::CgroupResourceProfilingBackend;
pub use cgroup_sampling_backend::CgroupSamplingBackend;
pub use common::managed::{
    ManagedRecoveryRecord, ManagedScopePersistence, ResourceFinalizationDraft,
    ResourceFinalizationPoll,
};
pub use common::sampler::{
    CgroupCounterSource, CgroupProcessRssConfig, CgroupScopeSampler, CgroupWorkloadSample,
};
pub use config::{CgroupBackendMode, CgroupProfilingConfig};
use implementations::{v1, v2};
pub use v1::{
    CgroupV1BindingPersistence, CgroupV1BindingRecovery, CgroupV1BindingSampler, CgroupV1Config,
    ManagedV1ScopeConfig,
};
pub use v2::{
    CgroupV2BindingConfig, CgroupV2BindingFinalization, CgroupV2BindingPersistence,
    CgroupV2BindingPolicy, CgroupV2BindingRecovery, CgroupV2Config, ManagedV2ScopeConfig,
};
