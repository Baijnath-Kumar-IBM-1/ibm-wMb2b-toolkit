//! Control-number counters (per environment).
//!
//! ISA and GS control numbers are allocated atomically: two concurrent
//! allocations can never receive the same number.

use serde::Serialize;

use crate::error::Result;
use crate::ids::EnvironmentId;

/// ISA13 and GS06 are limited to 9 digits; numbers wrap from this value to 1.
pub const MAX_CONTROL_NUMBER: u32 = 999_999_999;

/// Counter state. `isa_next` and `gs_next` are the numbers the next
/// allocation will hand out. `run_count` counts completed runs and feeds the
/// `{seq}` part of trace values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Counters {
    pub isa_next: u32,
    pub gs_next: u32,
    pub run_count: u64,
}

impl Default for Counters {
    fn default() -> Self {
        Self {
            isa_next: 1,
            gs_next: 1,
            run_count: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ControlNumbers {
    pub isa: u32,
    pub gs: u32,
}

/// Advances a control number by `by`, wrapping 999,999,999 to 1.
pub fn advance(current: u32, by: u32) -> u32 {
    let zero_based =
        (u64::from(current.max(1)) - 1 + u64::from(by)) % u64::from(MAX_CONTROL_NUMBER);
    // The modulo keeps the value below MAX_CONTROL_NUMBER, which fits in u32.
    u32::try_from(zero_based).unwrap_or(0) + 1
}

/// Storage port for counters.
pub trait CounterRepository: Send + Sync {
    /// Current state without consuming numbers (for previews). An environment
    /// that has never allocated returns `Counters::default()`.
    fn peek(&self, environment_id: &EnvironmentId) -> Result<Counters>;
    /// Atomically reserves `count` consecutive ISA and GS numbers and returns
    /// them in order. `count` must be between 1 and 1000.
    fn allocate(&self, environment_id: &EnvironmentId, count: u32) -> Result<Vec<ControlNumbers>>;
    /// Atomically increments the run count and returns the new value.
    fn increment_run_count(&self, environment_id: &EnvironmentId) -> Result<u64>;
}

/// Largest batch accepted by [`CounterRepository::allocate`].
pub const MAX_ALLOCATION: u32 = 1000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_wraps_at_nine_digits() {
        assert_eq!(advance(1, 1), 2);
        assert_eq!(advance(MAX_CONTROL_NUMBER, 1), 1);
        assert_eq!(advance(MAX_CONTROL_NUMBER - 1, 3), 2);
        assert_eq!(advance(5, 0), 5);
    }
}
