//! Opaque identifier types.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};

macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident, $label:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn generate() -> Self {
                Self(Uuid::new_v4().to_string())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(s: &str) -> Result<Self> {
                let parsed = Uuid::parse_str(s)
                    .map_err(|_| Error::validation($label, "must be a UUID"))?;
                Ok(Self(parsed.hyphenated().to_string()))
            }
        }

        impl TryFrom<String> for $name {
            type Error = Error;

            fn try_from(value: String) -> Result<Self> {
                value.parse()
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

uuid_id!(
    /// Identifies an environment.
    EnvironmentId,
    "environment_id"
);
uuid_id!(
    /// Identifies a partner.
    PartnerId,
    "partner_id"
);
uuid_id!(
    /// Identifies a stored submission.
    SubmissionId,
    "submission_id"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_round_trip() {
        let id = EnvironmentId::generate();
        assert_eq!(id.as_str().parse::<EnvironmentId>().unwrap(), id);
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!("not-a-uuid".parse::<PartnerId>().is_err());
        assert!("../../etc/passwd".parse::<PartnerId>().is_err());
    }

    #[test]
    fn deserialize_validates() {
        assert!(serde_json::from_str::<PartnerId>("\"nope\"").is_err());
        let id = PartnerId::generate();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<PartnerId>(&json).unwrap(), id);
    }
}
