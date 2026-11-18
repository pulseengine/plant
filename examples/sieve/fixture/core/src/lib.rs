// A module whose file sits outside core/: only a #[path] reaches it.
#[path = "../../shared/table.rs"]
mod table;

/// Limits read at compile time from a file outside core/.
pub const LIMITS: &str = include_str!("../../data/limits.txt");

pub fn lookup(i: usize) -> u8 {
    table::TABLE[i % table::TABLE.len()]
}
