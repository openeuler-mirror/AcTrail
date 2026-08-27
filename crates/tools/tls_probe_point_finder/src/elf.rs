//! ELF parser modules.

#[path = "elf/analysis_cache.rs"]
mod analysis_cache;
#[path = "elf/constants.rs"]
mod constants;
#[path = "elf/dynamic.rs"]
mod dynamic;
#[path = "elf/image.rs"]
mod image;
#[path = "elf/raw.rs"]
mod raw;
#[path = "elf/scan.rs"]
mod scan;
#[path = "elf/symbols.rs"]
mod symbols;

pub use analysis_cache::{BinaryAnalysisCache, BinaryAnalysisCacheStats};
pub(crate) use dynamic::DynamicInfo;
pub(crate) use image::{Arch, ElfImage};
pub(crate) use scan::DEFAULT_LOW_MEMORY_CHUNK_BYTES;
pub use scan::ScanMode;
