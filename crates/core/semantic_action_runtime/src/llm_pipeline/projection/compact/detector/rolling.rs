use super::super::state::{ResponseFingerprints, ResponseTextWitness};
use std::collections::HashMap;

pub(super) struct RollingWindowIndex<'a> {
    source: &'a [u8],
    data: &'a ResponseFingerprints,
}

pub(super) struct MatchedSpan {
    pub(super) response_start: usize,
    pub(super) response_end: usize,
    candidate_start: usize,
}

pub(super) struct MatchSearch {
    pub(super) best: Option<MatchedSpan>,
    pub(super) work_limit_exceeded: bool,
}

impl MatchedSpan {
    pub(super) fn len(&self) -> usize {
        self.response_end.saturating_sub(self.response_start)
    }

    fn covers_window(&self, source_offset: usize, candidate_offset: usize, window: usize) -> bool {
        let Some(source_relative) = source_offset.checked_sub(self.response_start) else {
            return false;
        };
        let Some(candidate_relative) = candidate_offset.checked_sub(self.candidate_start) else {
            return false;
        };
        source_relative == candidate_relative
            && self
                .len()
                .checked_sub(window)
                .is_some_and(|last_start| source_relative <= last_start)
    }
}

impl<'a> RollingWindowIndex<'a> {
    const BASE: u64 = 257;

    pub(super) fn new(
        response: &'a ResponseTextWitness,
        min_flow: usize,
        max_positions: usize,
    ) -> Self {
        let source = response.text().as_bytes();
        let data = response
            .content
            .fingerprints
            .get_or_init(|| Self::prepare(source, min_flow, max_positions));
        Self { source, data }
    }

    fn prepare(source: &[u8], min_flow: usize, max_positions: usize) -> ResponseFingerprints {
        let window = (min_flow / 2).max(1);
        let mut fingerprints = HashMap::<u64, Vec<usize>>::new();
        let mut offset = 0_usize;
        while offset + window <= source.len() {
            let hash = Self::hash(&source[offset..offset + window]);
            let positions = fingerprints.entry(hash).or_default();
            if positions.len() < max_positions {
                positions.push(offset);
            }
            offset = offset.saturating_add(window);
        }
        ResponseFingerprints {
            window,
            max_verified_hits: max_positions,
            positions: fingerprints,
        }
    }

    pub(super) fn longest_verified_match(&self, candidate: &[u8]) -> MatchSearch {
        if self.data.positions.is_empty() || candidate.len() < self.data.window {
            return MatchSearch {
                best: None,
                work_limit_exceeded: false,
            };
        }
        let power = Self::BASE.wrapping_pow((self.data.window - 1) as u32);
        let mut hash = Self::hash(&candidate[..self.data.window]);
        let mut best = None;
        let mut verified_hits = 0_usize;
        for offset in 0..=candidate.len() - self.data.window {
            if offset > 0 {
                hash = hash
                    .wrapping_sub(u64::from(candidate[offset - 1]).wrapping_mul(power))
                    .wrapping_mul(Self::BASE)
                    .wrapping_add(u64::from(candidate[offset + self.data.window - 1]));
            }
            let Some(source_offsets) = self.data.positions.get(&hash) else {
                continue;
            };
            for &source_offset in source_offsets {
                if verified_hits >= self.data.max_verified_hits {
                    return MatchSearch {
                        best,
                        work_limit_exceeded: true,
                    };
                }
                verified_hits += 1;
                // Aligned windows inside the best span are already byte-verified.
                // Keep charging each fingerprint hit against the existing budget.
                if best.as_ref().is_some_and(|span: &MatchedSpan| {
                    span.covers_window(source_offset, offset, self.data.window)
                }) {
                    continue;
                }
                if self.source[source_offset..source_offset + self.data.window]
                    != candidate[offset..offset + self.data.window]
                {
                    continue;
                }
                let span = self.expand(source_offset, candidate, offset);
                if span.len() == self.source.len() {
                    return MatchSearch {
                        best: Some(span),
                        work_limit_exceeded: false,
                    };
                }
                if best
                    .as_ref()
                    .is_none_or(|current: &MatchedSpan| span.len() > current.len())
                {
                    best = Some(span);
                }
            }
        }
        MatchSearch {
            best,
            work_limit_exceeded: false,
        }
    }

    fn expand(
        &self,
        source_offset: usize,
        candidate: &[u8],
        candidate_offset: usize,
    ) -> MatchedSpan {
        let mut source_start = source_offset;
        let mut candidate_start = candidate_offset;
        while source_start > 0
            && candidate_start > 0
            && self.source[source_start - 1] == candidate[candidate_start - 1]
        {
            source_start -= 1;
            candidate_start -= 1;
        }
        let mut source_end = source_offset + self.data.window;
        let mut candidate_end = candidate_offset + self.data.window;
        while source_end < self.source.len()
            && candidate_end < candidate.len()
            && self.source[source_end] == candidate[candidate_end]
        {
            source_end += 1;
            candidate_end += 1;
        }
        MatchedSpan {
            response_start: source_start,
            response_end: source_end,
            candidate_start,
        }
    }

    fn hash(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0_u64, |hash, byte| {
            hash.wrapping_mul(Self::BASE).wrapping_add(u64::from(*byte))
        })
    }
}
