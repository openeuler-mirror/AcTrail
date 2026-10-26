//! Transaction-local physical blocks for lossless logical event storage.

mod builder;
mod codec;

pub(crate) use builder::EventRecordBlockWriter;
pub(crate) use codec::{decode_block, decode_event, decode_kind_counts};
