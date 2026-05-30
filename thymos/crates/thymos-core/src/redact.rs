//! Secret-redaction boundary for data that crosses into the immutable ledger.
//!
//! Tool observations are persisted verbatim in `Commit` bodies, which are
//! append-only and content-addressed — once a secret lands there it cannot be
//! deleted without breaking the hash chain. A `Redactor` rewrites values under
//! sensitive keys *before* they reach the ledger (and, because the agent loop
//! re-reads observations from the ledger, before cognition sees them on the
//! next step). This trades raw-secret pass-through for the guarantee that
//! credentials are never written to permanent storage.

use serde_json::{Map, Value};

/// The marker substituted in place of a redacted value.
pub const REDACTED: &str = "***REDACTED***";

/// Rewrites JSON values whose object key matches a sensitive pattern.
///
/// Matching is case-insensitive substring matching against the key name, so
/// `apiKey`, `API_KEY`, and `x-api-key` all match the pattern `api_key`'s
/// component substrings. The default set covers the common credential-bearing
/// field names; deployments can supply their own with [`Redactor::with_keys`].
#[derive(Clone, Debug)]
pub struct Redactor {
    /// Lowercased substrings; a key is sensitive if it contains any of them.
    needles: Vec<String>,
}

impl Redactor {
    /// The standard credential-bearing key set, enabled by default.
    pub fn default_secrets() -> Self {
        Redactor {
            needles: [
                "password",
                "passwd",
                "secret",
                "token",
                "api_key",
                "apikey",
                "api-key",
                "authorization",
                "auth",
                "bearer",
                "credential",
                "private_key",
                "client_secret",
                "access_key",
                "session",
                "cookie",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        }
    }

    /// A no-op redactor that passes everything through unchanged.
    pub fn none() -> Self {
        Redactor { needles: Vec::new() }
    }

    /// Build a redactor from a custom set of (case-insensitive) key substrings.
    pub fn with_keys<I, S>(keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Redactor {
            needles: keys.into_iter().map(|s| s.into().to_lowercase()).collect(),
        }
    }

    /// True if `key` names a sensitive field.
    pub fn is_sensitive(&self, key: &str) -> bool {
        if self.needles.is_empty() {
            return false;
        }
        let k = key.to_lowercase();
        self.needles.iter().any(|n| k.contains(n.as_str()))
    }

    /// Return a copy of `value` with every sensitive field's value replaced by
    /// [`REDACTED`]. Recurses through objects and arrays. Sensitive keys are
    /// redacted regardless of how deeply nested their subtree is.
    pub fn redact(&self, value: &Value) -> Value {
        if self.needles.is_empty() {
            return value.clone();
        }
        match value {
            Value::Object(map) => {
                let mut out = Map::with_capacity(map.len());
                for (k, v) in map {
                    if self.is_sensitive(k) {
                        out.insert(k.clone(), Value::String(REDACTED.to_string()));
                    } else {
                        out.insert(k.clone(), self.redact(v));
                    }
                }
                Value::Object(out)
            }
            Value::Array(items) => Value::Array(items.iter().map(|v| self.redact(v)).collect()),
            other => other.clone(),
        }
    }
}

impl Default for Redactor {
    fn default() -> Self {
        Self::default_secrets()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_top_level_secret() {
        let r = Redactor::default_secrets();
        let out = r.redact(&json!({"api_key": "sk-123", "host": "example.com"}));
        assert_eq!(out["api_key"], REDACTED);
        assert_eq!(out["host"], "example.com");
    }

    #[test]
    fn redacts_case_insensitively_and_nested() {
        let r = Redactor::default_secrets();
        let out = r.redact(&json!({
            "headers": {"Authorization": "Bearer x", "Accept": "json"},
            "rows": [{"PASSWORD": "p"}, {"id": 1}]
        }));
        assert_eq!(out["headers"]["Authorization"], REDACTED);
        assert_eq!(out["headers"]["Accept"], "json");
        assert_eq!(out["rows"][0]["PASSWORD"], REDACTED);
        assert_eq!(out["rows"][1]["id"], 1);
    }

    #[test]
    fn none_passes_through() {
        let r = Redactor::none();
        let v = json!({"token": "keepme"});
        assert_eq!(r.redact(&v), v);
    }

    #[test]
    fn custom_keys_only() {
        let r = Redactor::with_keys(["pin"]);
        let out = r.redact(&json!({"pin": "0000", "token": "left"}));
        assert_eq!(out["pin"], REDACTED);
        assert_eq!(out["token"], "left", "non-listed key untouched");
    }
}
