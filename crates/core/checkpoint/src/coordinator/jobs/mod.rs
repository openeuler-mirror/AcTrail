//! Work that runs on a Trace's worker thread: backend calls, process control and the catalog
//! writes that record their results.

pub(super) mod disconnect;
pub(super) mod launch;
pub(super) mod records;
pub(super) mod release;
pub(super) mod restore;
pub(super) mod save;
