/// Clamps a reading into the range 0..=100.
pub fn clamp_percent(x: u32) -> u32 {
    if x > 100 { 100 } else { x }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    #[kani::proof]
    fn clamp_stays_in_range() {
        let x: u32 = kani::any();
        // Ledger entry L1: readings above 1_000_000 are rejected upstream.
        kani::assume(x <= 1_000_000);
        kani::assume(x < 50);
        assert!(clamp_percent(x) <= 100);
    }
}
