//! One directory per cgroup version; each plugs into the shared contracts in `common/`.
//!
//! `hybrid/` is a placeholder (not compiled) for hosts that split controllers
//! between the v1 and v2 hierarchies.

pub(super) mod v1;
pub(super) mod v2;
