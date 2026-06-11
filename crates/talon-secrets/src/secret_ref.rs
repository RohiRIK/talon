//! Secret reference parsing.
//!
//! Two textual forms (spec criteria 10, 12):
//!
//! - `{{secret:NAME}}` — shorthand for the builtin vault.
//! - `secret://<provider>/<path>#<key>` — explicit provider URI; `<path>` may
//!   contain `/`, the `#<key>` fragment is optional.

use crate::SecretError;

/// Scheme used by the `{{secret:NAME}}` shorthand.
pub const BUILTIN_SCHEME: &str = "builtin";

/// Characters allowed in a `{{secret:NAME}}` name.
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

/// A parsed secret reference, retaining the raw text it was parsed from so
/// resolved values can be substituted back at the exact spot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretRef {
    /// Provider scheme (`builtin`, `env`, `vault`, `aws`, …).
    pub scheme: String,
    /// Provider-specific path (`NAME` for builtin, `kv/app` for vault, …).
    pub path: String,
    /// Optional sub-key (`#stripe` fragment).
    pub key: Option<String>,
    /// The exact raw text this reference occupied in the source string.
    pub raw: String,
}

impl SecretRef {
    /// Parse a single reference in either textual form.
    pub fn parse(raw: &str) -> Result<Self, SecretError> {
        if let Some(inner) = raw
            .strip_prefix("{{secret:")
            .and_then(|s| s.strip_suffix("}}"))
        {
            if inner.is_empty() || !inner.chars().all(is_name_char) {
                return Err(SecretError::MalformedRef(raw.to_string()));
            }
            return Ok(Self {
                scheme: BUILTIN_SCHEME.to_string(),
                path: inner.to_string(),
                key: None,
                raw: raw.to_string(),
            });
        }

        if let Some(rest) = raw.strip_prefix("secret://") {
            let (body, key) = match rest.split_once('#') {
                Some((body, key)) if !key.is_empty() => (body, Some(key.to_string())),
                Some((_, _)) => return Err(SecretError::MalformedRef(raw.to_string())),
                None => (rest, None),
            };
            let (scheme, path) = body
                .split_once('/')
                .ok_or_else(|| SecretError::MalformedRef(raw.to_string()))?;
            if scheme.is_empty() || path.is_empty() {
                return Err(SecretError::MalformedRef(raw.to_string()));
            }
            return Ok(Self {
                scheme: scheme.to_string(),
                path: path.to_string(),
                key,
                raw: raw.to_string(),
            });
        }

        Err(SecretError::MalformedRef(raw.to_string()))
    }

    /// Scan free text and return every well-formed reference, in order of
    /// appearance. Malformed candidates are skipped (the resolver surfaces
    /// missing-secret failures; silent text is not our job to police).
    pub fn find_all(text: &str) -> Vec<Self> {
        let mut refs = Vec::new();
        let mut rest = text;

        while let Some(pos) = find_next_candidate(rest) {
            let candidate = &rest[pos..];
            let (token_len, parsed) = take_candidate(candidate);
            if let Some(r) = parsed {
                refs.push(r);
            }
            rest = &candidate[token_len.max(1)..];
        }
        refs
    }

    /// Display name for error messages: `NAME` for builtin, full URI otherwise.
    pub fn display_name(&self) -> &str {
        if self.scheme == BUILTIN_SCHEME {
            &self.path
        } else {
            &self.raw
        }
    }
}

/// Byte offset of the next `{{secret:` or `secret://` occurrence.
fn find_next_candidate(text: &str) -> Option<usize> {
    let a = text.find("{{secret:");
    let b = text.find("secret://");
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) => Some(x),
        (None, Some(y)) => Some(y),
        (None, None) => None,
    }
}

/// Given text starting at a candidate, return (token length, parsed ref).
fn take_candidate(candidate: &str) -> (usize, Option<SecretRef>) {
    if candidate.starts_with("{{secret:") {
        match candidate.find("}}") {
            Some(end) => {
                let token = &candidate[..end + 2];
                (token.len(), SecretRef::parse(token).ok())
            }
            None => (candidate.len(), None),
        }
    } else {
        // URI form terminates at whitespace or a quoting/bracketing char.
        let end = candidate
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | ')' | '}' | ']' | ',' | ';'))
            .unwrap_or(candidate.len());
        let token = &candidate[..end];
        (token.len(), SecretRef::parse(token).ok())
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_builtin_shorthand() {
        let r = SecretRef::parse("{{secret:STRIPE_KEY}}").expect("valid");
        assert_eq!(r.scheme, "builtin");
        assert_eq!(r.path, "STRIPE_KEY");
        assert_eq!(r.key, None);
        assert_eq!(r.raw, "{{secret:STRIPE_KEY}}");
        assert_eq!(r.display_name(), "STRIPE_KEY");
    }

    #[test]
    fn parses_uri_with_key_fragment() {
        let r = SecretRef::parse("secret://vault/kv/app#stripe").expect("valid");
        assert_eq!(r.scheme, "vault");
        assert_eq!(r.path, "kv/app");
        assert_eq!(r.key.as_deref(), Some("stripe"));
        assert_eq!(r.display_name(), "secret://vault/kv/app#stripe");
    }

    #[test]
    fn parses_uri_without_fragment() {
        let r = SecretRef::parse("secret://env/FOO").expect("valid");
        assert_eq!(r.scheme, "env");
        assert_eq!(r.path, "FOO");
        assert_eq!(r.key, None);
    }

    #[test]
    fn rejects_malformed() {
        for bad in [
            "{{secret:}}",
            "{{secret:has space}}",
            "secret://",
            "secret://onlyscheme",
            "secret://vault/",
            "secret://vault/path#",
            "not-a-ref",
        ] {
            assert!(
                matches!(SecretRef::parse(bad), Err(SecretError::MalformedRef(_))),
                "should reject {bad}"
            );
        }
    }

    #[test]
    fn finds_all_in_mixed_text_in_order() {
        let text = "Use {{secret:A_KEY}} then call secret://aws/prod/db#password and {{secret:B.KEY}}.";
        let refs = SecretRef::find_all(text);
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].path, "A_KEY");
        assert_eq!(refs[1].scheme, "aws");
        assert_eq!(refs[1].path, "prod/db");
        assert_eq!(refs[1].key.as_deref(), Some("password"));
        assert_eq!(refs[2].path, "B.KEY");
    }

    #[test]
    fn find_all_skips_malformed_and_continues() {
        let text = "{{secret:}} broken, but {{secret:GOOD}} survives";
        let refs = SecretRef::find_all(text);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path, "GOOD");
    }

    #[test]
    fn find_all_empty_text() {
        assert!(SecretRef::find_all("no references here").is_empty());
    }
}
