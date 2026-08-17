//! Payload parsing primitives shared by semantic action projection.

mod encoding;
pub(crate) mod http;
pub(crate) mod llm;
#[cfg(test)]
mod testing;
