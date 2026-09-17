use crate::GuestBootId;

/// PSI averages for one pressure line (`some` or `full`), in millipercent
/// (value x 1000). The kernel prints fractional values like `avg10=2.35` and
/// values can exceed 100% when multiple tasks stall at once, so millipercent
/// keeps both the fractional part and values > 100%.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PsiAverages {
    pub avg10_millipercent: u32,
    pub avg60_millipercent: u32,
    pub avg300_millipercent: u32,
}

/// Guest-wide memory pressure sampled from `/proc/pressure/memory`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GuestPressureSnapshot {
    pub guest_boot_id: GuestBootId,
    pub sampled_at_ms: u64,
    pub memory_some: PsiAverages,
    pub memory_full: PsiAverages,
}
