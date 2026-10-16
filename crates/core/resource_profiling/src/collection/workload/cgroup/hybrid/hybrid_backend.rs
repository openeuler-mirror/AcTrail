// TODO: Evaluate HybridBackend against workloads whose required metrics span
// both cgroup v1 and v2 controller hierarchies.
//
// TODO: Define metric ownership, cross-hierarchy identity verification, sampling
// time alignment, and per-source failure semantics before implementing the backend.
//
// TODO: Keep Hybrid configuration and lifecycle within this module; register the
// backend in the cgroup enum and factory once its behavior is implemented.
