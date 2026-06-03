//! Synchronous payload processor contract.

use crate::{CoreResult, PayloadDirection};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadContext<'a> {
    pub direction: PayloadDirection,
    pub provider: &'a str,
    pub symbol: &'a str,
    pub stream_key: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Decision {
    Allow,
    Block {
        reason: String,
    },
    ReplaceEqualLen {
        replacement: Vec<u8>,
        reason: String,
    },
}

pub trait SyncProcessor {
    fn decide(&mut self, context: &PayloadContext<'_>, payload: &[u8]) -> CoreResult<Decision>;
}
