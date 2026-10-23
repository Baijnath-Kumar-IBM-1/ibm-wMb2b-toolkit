//! Trace value generation.
//!
//! Trace values give each EDI run a unique set of business reference numbers.
//! They mirror `generate_trace_values` in the v1 script (branch `v1-bash`).
//!
//! The pattern language supports two substitutions:
//! - `{date}` → `YYYYMMDD` from the clock  
//! - `{seq}`  → `run_count` formatted as a 4-digit zero-padded integer
//!
//! All keys in a pack's `[trace_rules]` table are expanded, then callers may
//! apply user overrides on top.

use std::collections::BTreeMap;

use time::OffsetDateTime;

use crate::clock::Clock;

/// A fully resolved set of `{{PLACEHOLDER}}` → `value` substitutions.
///
/// This is a plain string map. After generation the caller may mutate entries
/// to apply user-supplied overrides before passing to the renderer.
#[derive(Debug, Clone, Default)]
pub struct TraceValues(pub BTreeMap<String, String>);

impl TraceValues {
    /// Insert or overwrite a value.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.0.insert(key.into(), value.into());
    }

    /// Look up a value by key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
}

impl From<BTreeMap<String, String>> for TraceValues {
    fn from(m: BTreeMap<String, String>) -> Self {
        Self(m)
    }
}

/// Generate trace values from a pack's `trace_rules`, the current clock and
/// the run count.
///
/// `run_count` is the value from [`crate::counters::CounterRepository::peek`]
/// **before** incrementing (i.e. the next run will use `run_count + 1`); pass
/// the already-incremented value to match the v1 script's behaviour (it
/// increments before generating).
pub fn generate_trace_values(
    trace_rules: &BTreeMap<String, String>,
    clock: &dyn Clock,
    run_count: u64,
) -> TraceValues {
    let now: OffsetDateTime = clock.now();
    let date_stamp = format!(
        "{:04}{:02}{:02}",
        now.year(),
        u8::from(now.month()),
        now.day()
    );
    let seq = format!("{:04}", run_count % 10_000);

    let resolved = trace_rules
        .iter()
        .map(|(k, pattern)| {
            let v = pattern
                .replace("{date}", &date_stamp)
                .replace("{seq}", &seq);
            (k.clone(), v)
        })
        .collect();

    TraceValues(resolved)
}

/// Compute the delivery date string (`YYYYMMDD`) as +14 calendar days from
/// the clock, matching the v1 script's `date -v+14d +%Y%m%d`.
pub fn delivery_date(clock: &dyn Clock) -> String {
    use time::Duration;
    let dt = clock.now() + Duration::days(14);
    format!("{:04}{:02}{:02}", dt.year(), u8::from(dt.month()), dt.day())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use time::{Duration, OffsetDateTime};

    use super::*;
    use crate::testing::FixedClock;

    fn fixed_clock_2025() -> FixedClock {
        // 2025-06-15T10:30:00Z
        let base = OffsetDateTime::UNIX_EPOCH
            + Duration::days(20_254)
            + Duration::hours(10)
            + Duration::minutes(30);
        FixedClock::new(base)
    }

    #[test]
    fn retail_trace_values_match_script_patterns() {
        let clock = fixed_clock_2025();
        let mut rules = BTreeMap::new();
        rules.insert("RFQ_NUMBER".into(), "RFQ-{date}-{seq}".into());
        rules.insert("PO_NUMBER".into(), "PO-{date}-{seq}".into());

        let tv = generate_trace_values(&rules, &clock, 1);
        assert_eq!(tv.get("RFQ_NUMBER"), Some("RFQ-20250615-0001"));
        assert_eq!(tv.get("PO_NUMBER"), Some("PO-20250615-0001"));
    }

    #[test]
    fn seq_is_zero_padded_to_four_digits() {
        let clock = fixed_clock_2025();
        let mut rules = BTreeMap::new();
        rules.insert("X".into(), "{seq}".into());

        assert_eq!(
            generate_trace_values(&rules, &clock, 1).get("X"),
            Some("0001")
        );
        assert_eq!(
            generate_trace_values(&rules, &clock, 42).get("X"),
            Some("0042")
        );
        assert_eq!(
            generate_trace_values(&rules, &clock, 9999).get("X"),
            Some("9999")
        );
        // Wraps at 10_000.
        assert_eq!(
            generate_trace_values(&rules, &clock, 10_000).get("X"),
            Some("0000")
        );
    }

    #[test]
    fn delivery_date_is_14_days_ahead() {
        let clock = fixed_clock_2025();
        // 2025-06-15 + 14 = 2025-06-29
        assert_eq!(delivery_date(&clock), "20250629");
    }
}
