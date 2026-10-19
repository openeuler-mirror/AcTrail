pub use crate::VerifiedProcessTarget;
pub struct ProcfsProcessLiveness;
impl ProcfsProcessLiveness {
    pub fn is_gone(target: &VerifiedProcessTarget) -> Result<bool, String> {
        super::process_counter_reader::ProcessCounterReader::is_gone(target)
    }
}
