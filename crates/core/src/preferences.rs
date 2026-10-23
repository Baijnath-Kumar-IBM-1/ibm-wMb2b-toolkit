//! Application preferences.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::validation::Collector;

/// Defaults mirror the v1 script (`INTER_DOCUMENT_DELAY=2`,
/// `DEFAULT_TIMEOUT=30`, `POST_PAYLOAD_TIMEOUT=120`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub inter_document_delay_secs: u32,
    pub request_timeout_secs: u32,
    pub payload_timeout_secs: u32,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            inter_document_delay_secs: 2,
            request_timeout_secs: 30,
            payload_timeout_secs: 120,
        }
    }
}

impl Preferences {
    pub fn validate(&self) -> Result<()> {
        let mut c = Collector::new();
        if self.inter_document_delay_secs > 300 {
            c.add("inter_document_delay_secs", "must be between 0 and 300");
        }
        if !(1..=600).contains(&self.request_timeout_secs) {
            c.add("request_timeout_secs", "must be between 1 and 600");
        }
        if !(1..=1800).contains(&self.payload_timeout_secs) {
            c.add("payload_timeout_secs", "must be between 1 and 1800");
        }
        c.finish()
    }
}

/// Storage port for preferences.
pub trait PreferenceRepository: Send + Sync {
    /// Stored preferences, or defaults when nothing was saved.
    fn get(&self) -> Result<Preferences>;
    fn set(&self, preferences: &Preferences) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        assert!(Preferences::default().validate().is_ok());
    }

    #[test]
    fn out_of_range_is_rejected() {
        let p = Preferences {
            request_timeout_secs: 0,
            ..Preferences::default()
        };
        assert!(p.validate().is_err());
    }
}
