//! Input validation helpers.
//!
//! Validators return `Result<T, String>` where the string is a user-safe
//! message. [`Collector`] gathers per-field results into one
//! [`Error::Validation`].

use crate::error::{Error, FieldError, Result};

pub type Check<T> = std::result::Result<T, String>;

/// Collects field errors so the user sees all problems at once.
#[derive(Debug, Default)]
pub struct Collector(Vec<FieldError>);

impl Collector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the error for `field` (if any) and returns the value on success.
    pub fn field<T>(&mut self, field: &str, check: Check<T>) -> Option<T> {
        match check {
            Ok(v) => Some(v),
            Err(message) => {
                self.0.push(FieldError::new(field, message));
                None
            }
        }
    }

    pub fn add(&mut self, field: &str, message: impl Into<String>) {
        self.0.push(FieldError::new(field, message));
    }

    pub fn finish(self) -> Result<()> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(Error::Validation(self.0))
        }
    }
}

fn length_message(min: usize, max: usize) -> String {
    if min <= 1 {
        format!("must be at most {max} characters (and not empty)")
    } else {
        format!("must be between {min} and {max} characters")
    }
}

/// Trimmed free text without control characters.
pub fn text(value: &str, min: usize, max: usize) -> Check<String> {
    let v = value.trim();
    let len = v.chars().count();
    if len < min.max(1) || len > max {
        return Err(length_message(min, max));
    }
    if v.chars().any(char::is_control) {
        return Err("must not contain control characters".into());
    }
    Ok(v.to_string())
}

/// Trimmed text restricted to characters accepted by `allowed`.
pub fn restricted_text(
    value: &str,
    min: usize,
    max: usize,
    allowed: impl Fn(char) -> bool,
    hint: &str,
) -> Check<String> {
    let v = text(value, min, max)?;
    if v.chars().all(allowed) {
        Ok(v)
    } else {
        Err(format!("may only contain {hint}"))
    }
}

/// Digits only.
pub fn digits(value: &str, min: usize, max: usize) -> Check<String> {
    let v = value.trim();
    let len = v.chars().count();
    if len < min || len > max {
        return Err(format!("must be {min} to {max} digits"));
    }
    if !v.bytes().all(|b| b.is_ascii_digit()) {
        return Err("must contain digits only".into());
    }
    Ok(v.to_string())
}

/// Canonical lowercase hyphenated UUID.
pub fn uuid(value: &str) -> Check<String> {
    uuid::Uuid::parse_str(value.trim())
        .map(|u| u.hyphenated().to_string())
        .map_err(|_| "must be a UUID such as 123e4567-e89b-12d3-a456-426614174000".to_string())
}

/// Absolute file-system path (no existence check).
pub fn absolute_path(value: &str) -> Check<String> {
    let v = value.trim();
    if v.is_empty() || v.len() > 1024 || v.chars().any(char::is_control) {
        return Err("must be a path of at most 1024 characters".into());
    }
    if !std::path::Path::new(v).is_absolute() {
        return Err("must be an absolute path".into());
    }
    Ok(v.to_string())
}

/// Host name (optionally with `:port`) without scheme, path or credentials.
/// Returns the lowercase form.
pub fn host(value: &str, allow_port: bool) -> Check<String> {
    let v = value.trim();
    if v.is_empty() {
        return Err("is required".into());
    }
    if v.contains("://") {
        return Err("must not include a scheme such as https://".into());
    }
    if v.chars()
        .any(|c| c.is_whitespace() || c.is_control() || "/?#@\\".contains(c))
    {
        return Err("must be a host name only, without path or credentials".into());
    }
    let (name, port) = match v.matches(':').count() {
        0 => (v, None),
        1 if allow_port => {
            let (n, p) = v.split_once(':').unwrap_or((v, ""));
            (n, Some(p))
        }
        _ => return Err("must be a host name without a port".into()),
    };
    if let Some(p) = port {
        match p.parse::<u16>() {
            Ok(n) if n > 0 && p.bytes().all(|b| b.is_ascii_digit()) => {}
            _ => return Err("port must be between 1 and 65535".into()),
        }
    }
    if name.len() > 253 {
        return Err("is too long".into());
    }
    for label in name.split('.') {
        let ok = !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !ok {
            return Err("is not a valid host name".into());
        }
    }
    Ok(v.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_accepts_valid() {
        assert_eq!(
            host("Dev2205134-M-YQB-C1.b2b.ipaas.ibm.com", true).unwrap(),
            "dev2205134-m-yqb-c1.b2b.ipaas.ibm.com"
        );
        assert_eq!(host("127.0.0.1:8443", true).unwrap(), "127.0.0.1:8443");
        assert_eq!(host("localhost", true).unwrap(), "localhost");
    }

    #[test]
    fn host_rejects_invalid() {
        for bad in [
            "",
            "https://a.example.com",
            "a.example.com/path",
            "user@a.example.com",
            "a.example.com:0",
            "a.example.com:99999",
            "a.example.com:",
            "-a.example.com",
            "a..example.com",
            "a.example.com.",
            "a b.example.com",
            "a_b.example.com",
            "::1",
        ] {
            assert!(host(bad, true).is_err(), "{bad:?} should be rejected");
        }
        assert!(host("a.example.com:443", false).is_err());
    }

    #[test]
    fn uuid_normalises() {
        assert_eq!(
            uuid(" 123E4567-E89B-12D3-A456-426614174000 ").unwrap(),
            "123e4567-e89b-12d3-a456-426614174000"
        );
        assert!(uuid("abc").is_err());
    }

    #[test]
    fn digits_and_text() {
        assert!(digits("777888900", 9, 15).is_ok());
        assert!(digits("77788890", 9, 15).is_err());
        assert!(digits("77788890a", 9, 15).is_err());
        assert!(text("  ", 1, 5).is_err());
        assert!(text("a\u{7}b", 1, 5).is_err());
        assert_eq!(text(" ab ", 1, 5).unwrap(), "ab");
    }

    #[test]
    fn path_must_be_absolute() {
        assert!(absolute_path("/etc/ssl/ca.pem").is_ok());
        assert!(absolute_path("ca.pem").is_err());
        assert!(absolute_path("").is_err());
    }

    #[test]
    fn collector_gathers_all_errors() {
        let mut c = Collector::new();
        c.field("a", Err::<(), _>("bad".into()));
        c.field("b", Err::<(), _>("worse".into()));
        match c.finish() {
            Err(Error::Validation(v)) => assert_eq!(v.len(), 2),
            other => panic!("unexpected {other:?}"),
        }
    }
}
