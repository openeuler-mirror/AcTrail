//! Resource profiling backends with locally owned configuration and sampling state.

mod config;
pub use collection::contracts::ProfilingError;
pub use collection::workload::{
    CgroupCounterSource, CgroupResourceProfilingBackend, CgroupWorkloadSample,
};

pub use collection::contracts::{ResourceProfilingBackend, SamplingContext};
pub use collection::host::{
    HostProfiler, HostResourceProfilingBackend, LinuxHostResourceProfilingBackend,
};
pub use collection::workload::WorkloadResourceProfilingBackend;

pub use collection::host::{HostBackendFactory, HostBackendMode, HostProfilingConfig};
pub use collection::workload::{
    CgroupProcessRssConfig, CgroupProfilingConfig, CgroupV2BindingConfig, CgroupV2BindingPolicy,
    ManagedV2ScopeConfig, WorkloadBackendMode, WorkloadProfilingConfig,
};
pub use config::ResourceProfilingConfig;

pub use collection::workload::{
    CgroupPersistence, CgroupV2BindingFinalization, ResourceFinalizationDraft,
    ResourceFinalizationPoll, ResourceMetricsDrain, ResourceSampleDraft, ResourceSamplingFailure,
    WorkloadProfiler, WorkloadTarget, WorkloadTargets,
};
pub use collection::workload::{CgroupV2BindingPersistence, CgroupV2BindingRecovery};
pub use collection::workload::{ManagedRecoveryRecord, ManagedScopePersistence};
pub use collection::workload::{
    ProcfsProcessLiveness, ProcfsWorkloadProfilingConfig, ProcfsWorkloadResourceProfilingBackend,
    ProcfsWorkloadSample,
};
pub use collection::workload::{
    SamplingSelection, VerifiedProcessTarget, WorkloadBackend, WorkloadBackendFactory,
    WorkloadSamplingBackend,
};

pub use collection::workload::{
    CgroupBackendMode, CgroupScopeSampler, CgroupV1BindingPersistence, CgroupV1BindingRecovery,
    CgroupV1BindingSampler, CgroupV1Config, CgroupV2Config, ManagedV1ScopeConfig,
};

pub mod alerts;
mod collection;
pub mod model;
pub mod storage;
pub mod timeline;
pub use collection::ResourceCollectionConfig;
mod service;
pub use service::{ResourceCompletion, ResourceCompletionPoll};
pub use service::{ResourceProfilingFactory, ResourceProfilingService};
