//! Bounded structural discovery of aarch64 rustls traffic-start functions.
mod analysis;
mod frames;
mod instruction;
mod limits;
mod patterns;

use std::collections::{BTreeMap, BTreeSet};

use crate::elf::ElfImage;
use crate::probe_detector::contract::detector::DetectorConfigError;
use analysis::FunctionAnalysis;
use frames::FunctionLookup;
use limits::Limits;
use patterns::CandidatePatterns;

#[derive(Clone, Copy, Debug)]
pub(super) struct HandshakeLocation {
    pub(super) virtual_address: u64,
    pub(super) send_field: u16,
    // This is structural caller evidence, not independent proof of receive semantics.
    pub(super) preceding_field: u16,
}

#[derive(Clone, Copy, Debug)]
#[repr(u64)]
pub(super) enum Rejection {
    MissingBoundaries = 1,
    FunctionBudget,
    DataflowBudget,
    NoSendField,
    CandidateBudget = 6,
    CallBudget = 8,
    NoCandidate,
    Ambiguous,
    InvalidImage = 12,
    PrefixBudget,
}

impl Rejection {
    fn from_code(code: u64) -> Self {
        match code {
            1 => Self::MissingBoundaries,
            2 => Self::FunctionBudget,
            3 => Self::DataflowBudget,
            4 => Self::NoSendField,
            6 => Self::CandidateBudget,
            8 => Self::CallBudget,
            9 => Self::NoCandidate,
            10 => Self::Ambiguous,
            13 => Self::PrefixBudget,
            _ => Self::InvalidImage,
        }
    }
}

pub(super) struct RustlsHandshakeLocator {
    limits: Limits,
}

impl RustlsHandshakeLocator {
    pub(super) fn try_new() -> Result<Self, DetectorConfigError> {
        Ok(Self {
            limits: Limits::from_environment()?,
        })
    }

    pub(super) fn register(&self, image: &ElfImage) {
        // Boundary metadata is a prerequisite; reject before shared pattern scanning.
        if FunctionLookup::new(image).is_ok() {
            CandidatePatterns::register(image);
        }
    }

    pub(super) fn locate(
        &self,
        image: &ElfImage,
        plaintext: u64,
    ) -> Result<HandshakeLocation, Rejection> {
        let key = format!("rustls-handshake-v3:{plaintext:x}:{:?}", self.limits);
        if let Some(Some(cached)) = image.cached_named_addresses(&key) {
            if let Some(code) = cached.get("rejection") {
                return Err(Rejection::from_code(*code));
            }
            if let (Some(address), Some(send), Some(preceding)) = (
                cached.get("entry"),
                cached.get("send"),
                cached.get("preceding"),
            ) {
                return Ok(HandshakeLocation {
                    virtual_address: *address,
                    send_field: *send as u16,
                    preceding_field: *preceding as u16,
                });
            }
            return Err(Rejection::InvalidImage);
        }
        let result = self.scan(image, plaintext);
        let cached = match result {
            Ok(found) => BTreeMap::from([
                ("entry".into(), found.virtual_address),
                ("send".into(), u64::from(found.send_field)),
                ("preceding".into(), u64::from(found.preceding_field)),
            ]),
            Err(reason) => BTreeMap::from([("rejection".into(), reason as u64)]),
        };
        image.cache_named_addresses(key, Some(cached));
        result
    }

    fn scan(&self, image: &ElfImage, plaintext: u64) -> Result<HandshakeLocation, Rejection> {
        let frames = FunctionLookup::new(image)?;
        let (start, end) = frames.range(plaintext)?;
        if start != plaintext {
            return Err(Rejection::MissingBoundaries);
        }
        let mut budget = self.limits.dataflow_steps;
        let fields = FunctionAnalysis::new(
            start,
            frames.words(start, end, self.limits.function_instructions)?,
        )
        .send_fields(&mut budget)?;
        let patterns = CandidatePatterns::new(image)?;
        let mut functions = BTreeSet::new();
        for address in patterns.handshake_entries() {
            functions.insert(frames.range(address?)?);
            if functions.len() > self.limits.candidates {
                return Err(Rejection::CandidateBudget);
            }
        }
        let mut candidates = Vec::new();
        for (start, end) in functions {
            let prefix_end = end.min(
                start.saturating_add((self.limits.prefix_instructions as u64).saturating_mul(4)),
            );
            let prefix = FunctionAnalysis::new(
                start,
                frames.words(start, prefix_end, self.limits.prefix_instructions)?,
            );
            let Some(prefix) =
                prefix.prefix(&fields, self.limits.prefix_instructions, &mut budget)?
            else {
                continue;
            };
            let function = FunctionAnalysis::new(
                start,
                frames.words(start, end, self.limits.function_instructions)?,
            );
            if let Some(found) = function.handshake(prefix, &mut budget)? {
                candidates.push((found, BTreeSet::new()));
            }
        }
        if candidates.is_empty() {
            return Err(Rejection::NoCandidate);
        }
        let mut calls = 0usize;
        patterns.visit_calls(|pc, target| {
            let Some((candidate, caller_fields)) = candidates
                .iter_mut()
                .find(|(candidate, _)| candidate.virtual_address == target)
            else {
                return Ok(());
            };
            calls += 1;
            if calls > self.limits.matching_calls {
                return Err(Rejection::CallBudget);
            }
            let (start, _) = frames.range(pc)?;
            let window = (self.limits.caller_instructions as u64).saturating_mul(4);
            let start = start.max(pc.saturating_sub(window));
            let function = FunctionAnalysis::new(
                start,
                frames.words(
                    start,
                    pc + 4,
                    self.limits.caller_instructions.saturating_add(1),
                )?,
            );
            for field in function.preceding_fields(&mut budget)? {
                if field != candidate.send_field {
                    caller_fields.insert(field);
                }
            }
            Ok(())
        })?;
        let mut accepted = candidates
            .into_iter()
            .filter_map(|(mut candidate, fields)| {
                if fields.len() != 1 {
                    return None;
                }
                candidate.preceding_field = *fields.first()?;
                Some(candidate)
            });
        let found = accepted.next().ok_or(Rejection::NoCandidate)?;
        if accepted.next().is_some() {
            return Err(Rejection::Ambiguous);
        }
        Ok(found)
    }
}
