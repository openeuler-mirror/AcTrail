//! Pattern scanning strategies owned by the ELF image.
//!
//! Detectors stay agnostic to whether the image is scanned with full-memory
//! slices or a low-memory chunked pass; `ElfImage` dispatches to the selected
//! strategy.

use memmap2::{Advice, Mmap};

use crate::pattern_search::ExactPatternSearch;

pub(crate) const DEFAULT_LOW_MEMORY_CHUNK_BYTES: usize = 32 * 1024 * 1024;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ScanMode {
    Full,
    LowMemory,
}

/// Shared executable pattern scan results.
///
/// Pattern-based TLS detectors register every pattern they may need before
/// any scan happens; the first request runs one combined pass over the
/// executable ranges (a single low-memory touch) and every detector then
/// reads its offsets from the cached result.
#[derive(Default)]
pub(crate) struct PatternScanCache {
    patterns: Vec<Vec<u8>>,
    scanned: Vec<Option<Vec<usize>>>,
}

impl PatternScanCache {
    pub(crate) fn register(&mut self, pattern: &[u8]) -> usize {
        if let Some(index) = self.patterns.iter().position(|cached| cached == pattern) {
            return index;
        }
        self.patterns.push(pattern.to_vec());
        self.scanned.push(None);
        self.patterns.len() - 1
    }

    pub(crate) fn scan_if_needed(
        &mut self,
        ranges: &[(usize, &[u8])],
        mode: ScanMode,
        mmap: Option<&Mmap>,
        chunk_bytes: usize,
    ) -> bool {
        let pending = self
            .scanned
            .iter()
            .enumerate()
            .filter_map(|(index, offsets)| offsets.is_none().then_some(index))
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return false;
        }
        let pending_patterns = pending
            .iter()
            .map(|&index| self.patterns[index].as_slice())
            .collect::<Vec<_>>();
        let results = find_all_multi(ranges, &pending_patterns, mode, mmap, chunk_bytes);
        for (index, offsets) in pending.into_iter().zip(results) {
            self.scanned[index] = Some(offsets);
        }
        true
    }

    pub(crate) fn offsets(&self, index: usize) -> &[usize] {
        self.scanned[index].as_deref().unwrap_or_default()
    }
}

pub(crate) fn find_all_multi(
    ranges: &[(usize, &[u8])],
    patterns: &[&[u8]],
    mode: ScanMode,
    mmap: Option<&Mmap>,
    chunk_bytes: usize,
) -> Vec<Vec<usize>> {
    match mode {
        ScanMode::Full => find_all_multi_full(ranges, patterns),
        ScanMode::LowMemory => {
            let mmap = mmap.expect("low-memory scan requires the owning mmap");
            find_all_multi_low_memory(mmap, ranges, patterns, chunk_bytes)
        }
    }
}

fn find_all_multi_full(ranges: &[(usize, &[u8])], patterns: &[&[u8]]) -> Vec<Vec<usize>> {
    patterns
        .iter()
        .map(|pattern| {
            ExactPatternSearch::new(pattern)
                .map_or_else(Vec::new, |search| search.find_all_in_file_ranges(ranges))
        })
        .collect()
}

fn find_all_multi_low_memory(
    mmap: &Mmap,
    ranges: &[(usize, &[u8])],
    patterns: &[&[u8]],
    chunk_bytes: usize,
) -> Vec<Vec<usize>> {
    // Drop pages touched by ELF parsing / mmap readahead that fall outside the
    // scanned ranges (e.g. file headers before the first executable section),
    // so the low-memory pass only keeps the chunks it is scanning.
    let _ = mmap.advise_range(Advice::DontNeed, 0, mmap.len());
    let searches = patterns
        .iter()
        .map(|pattern| ExactPatternSearch::new(pattern))
        .collect::<Vec<_>>();
    let mut found = vec![Vec::new(); patterns.len()];
    let chunk_bytes = chunk_bytes.max(1);
    let max_pattern_length = patterns
        .iter()
        .map(|pattern| pattern.len())
        .max()
        .unwrap_or(0);
    let overlap = max_pattern_length.saturating_sub(1);

    for &(range_offset, range_data) in ranges {
        if max_pattern_length > range_data.len() {
            continue;
        }
        let mut start = 0;
        while start < range_data.len() {
            let chunk_end = (start + chunk_bytes + overlap).min(range_data.len());
            let chunk = &range_data[start..chunk_end];
            for (index, search) in searches.iter().enumerate() {
                let Some(search) = search else {
                    continue;
                };
                for relative in search.find_all(chunk) {
                    found[index].push(range_offset + start + relative);
                }
            }
            let release_end = chunk_end.saturating_sub(overlap);
            if release_end > start {
                let _ =
                    mmap.advise_range(Advice::DontNeed, range_offset + start, release_end - start);
            }
            start = release_end;
            if chunk_end >= range_data.len() {
                break;
            }
        }
    }

    for offsets in &mut found {
        offsets.sort_unstable();
        offsets.dedup();
    }
    found
}
