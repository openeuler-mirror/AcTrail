//! TLS provider detection modules.

mod command;
mod report;

pub(crate) use command::run;
pub(crate) use report::{
    CandidateReport, CandidateStatus, DetectReport, ExportedSymbolReport, MapStatusReport,
    OffsetAddressReport, PatternMatchesReport, SymbolMapReport, TargetReport,
};
