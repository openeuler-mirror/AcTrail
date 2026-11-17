//! Process identity observation, coordinate resolution, and logical-ID management.

mod contract;
mod manager;

pub use contract::{IdentityLookupError, ProcessIdentityReader};
pub use manager::{
    HostProcessLookup, ProcessIdentityError, ProcessIdentityManager, ProcessResolution,
};
