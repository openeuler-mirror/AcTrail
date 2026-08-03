//! Optimized exact byte-pattern search shared by static probe detectors.

use memchr::memmem::Finder;

pub(crate) struct ExactPatternSearch<'pattern> {
    finder: Finder<'pattern>,
    pattern_length: usize,
}

impl<'pattern> ExactPatternSearch<'pattern> {
    pub(crate) fn new(pattern: &'pattern [u8]) -> Option<Self> {
        (!pattern.is_empty()).then(|| Self {
            finder: Finder::new(pattern),
            pattern_length: pattern.len(),
        })
    }

    pub(crate) fn find_all(&self, data: &[u8]) -> Vec<usize> {
        if self.pattern_length > data.len() {
            return Vec::new();
        }
        let mut offsets = Vec::new();
        let mut start = 0;
        while start <= data.len() - self.pattern_length {
            let Some(relative) = self.finder.find(&data[start..]) else {
                break;
            };
            let offset = start + relative;
            offsets.push(offset);
            start = offset + 1;
        }
        offsets
    }

    pub(crate) fn find_all_in_file_ranges(&self, ranges: &[(usize, &[u8])]) -> Vec<usize> {
        let mut offsets = ranges
            .iter()
            .flat_map(|(file_offset, data)| {
                let file_offset = *file_offset;
                self.find_all(data)
                    .into_iter()
                    .map(move |relative| file_offset + relative)
            })
            .collect::<Vec<_>>();
        offsets.sort_unstable();
        offsets.dedup();
        offsets
    }
}
