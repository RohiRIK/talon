//! Skill manifest — the **host-trusted** sidecar (`<name>.toml`) that sits beside
//! each `<name>.wasm`.
//!
//! Why a sidecar and not a `.wasm` export: the manifest declares the skill's
//! `approval_level` and its capability allowlist. Untrusted guest code must never
//! be able to assert its own danger level or grant itself capabilities, so this
//! metadata lives outside the sandbox where the host alone authors it.

use serde::Deserialize;
use talon_core::approval::ApprovalLevel;

/// Parsed, validated skill metadata. `capabilities` is the allowlist the
/// [`crate::host`] sandbox enforces: a host import whose capability is absent
/// here traps when the guest calls it.
#[derive(Debug, Clone)]
pub struct SkillManifest {
    pub name: String,
    pub description: String,
    pub approval_level: ApprovalLevel,
    pub capabilities: Vec<String>,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("failed to read manifest {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid manifest TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("manifest 'name' must be non-empty")]
    EmptyName,
    #[error("unknown approval_level '{0}' — expected safe | needs_approval | dangerous")]
    BadApproval(String),
}

#[derive(Deserialize)]
struct RawManifest {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_approval")]
    approval_level: String,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    input_schema: Option<serde_json::Value>,
}

fn default_approval() -> String {
    "needs_approval".to_string()
}

fn default_schema() -> serde_json::Value {
    serde_json::json!({ "type": "object" })
}

impl SkillManifest {
    /// Parse and validate a manifest from TOML text.
    pub fn from_toml(text: &str) -> Result<Self, ManifestError> {
        let raw: RawManifest = toml::from_str(text)?;
        if raw.name.trim().is_empty() {
            return Err(ManifestError::EmptyName);
        }
        let approval_level = parse_approval(&raw.approval_level)?;
        Ok(Self {
            name: raw.name,
            description: raw.description,
            approval_level,
            capabilities: raw.capabilities,
            input_schema: raw.input_schema.unwrap_or_else(default_schema),
        })
    }

    /// Read and parse the sidecar manifest at `path`.
    pub fn load(path: &std::path::Path) -> Result<Self, ManifestError> {
        let text = std::fs::read_to_string(path).map_err(|source| ManifestError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_toml(&text)
    }

    /// Does this skill's allowlist grant `capability`?
    pub fn grants(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }
}

/// Map the lowercase manifest string to the core [`ApprovalLevel`]. Strict —
/// an unknown value is a hard error, never silently downgraded.
fn parse_approval(s: &str) -> Result<ApprovalLevel, ManifestError> {
    match s.trim().to_ascii_lowercase().as_str() {
        "safe" => Ok(ApprovalLevel::Safe),
        "needs_approval" | "needsapproval" => Ok(ApprovalLevel::NeedsApproval),
        "dangerous" => Ok(ApprovalLevel::Dangerous),
        other => Err(ManifestError::BadApproval(other.to_string())),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_manifest() {
        let m = SkillManifest::from_toml(
            r#"
            name = "hello"
            description = "Echo a greeting"
            approval_level = "safe"
            capabilities = ["log"]
            "#,
        )
        .expect("parse");
        assert_eq!(m.name, "hello");
        assert_eq!(m.description, "Echo a greeting");
        assert_eq!(m.approval_level, ApprovalLevel::Safe);
        assert!(m.grants("log"));
        assert!(!m.grants("network"));
    }

    #[test]
    fn approval_defaults_to_needs_approval() {
        let m = SkillManifest::from_toml(r#"name = "x""#).expect("parse");
        assert_eq!(m.approval_level, ApprovalLevel::NeedsApproval);
        assert!(m.capabilities.is_empty());
    }

    #[test]
    fn schema_defaults_to_object() {
        let m = SkillManifest::from_toml(r#"name = "x""#).expect("parse");
        assert_eq!(m.input_schema, serde_json::json!({ "type": "object" }));
    }

    #[test]
    fn custom_input_schema_is_preserved() {
        let m = SkillManifest::from_toml(
            r#"
            name = "x"
            [input_schema]
            type = "object"
            [input_schema.properties.msg]
            type = "string"
            "#,
        )
        .expect("parse");
        assert_eq!(m.input_schema["properties"]["msg"]["type"], "string");
    }

    #[test]
    fn empty_name_is_rejected() {
        let err = SkillManifest::from_toml(r#"name = "  ""#).unwrap_err();
        assert!(matches!(err, ManifestError::EmptyName));
    }

    #[test]
    fn unknown_approval_is_rejected() {
        let err = SkillManifest::from_toml(
            r#"
            name = "x"
            approval_level = "yolo"
            "#,
        )
        .unwrap_err();
        assert!(matches!(err, ManifestError::BadApproval(_)));
    }

    #[test]
    fn dangerous_and_needs_approval_aliases_parse() {
        assert_eq!(
            parse_approval("dangerous").unwrap(),
            ApprovalLevel::Dangerous
        );
        assert_eq!(
            parse_approval("NeedsApproval").unwrap(),
            ApprovalLevel::NeedsApproval
        );
        assert_eq!(parse_approval(" Safe ").unwrap(), ApprovalLevel::Safe);
    }
}
