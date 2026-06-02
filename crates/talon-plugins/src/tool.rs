//! Adapter: present a compiled WASM skill as an [`Arc<dyn Tool>`] so the agent
//! registry treats it exactly like a built-in tool.
//!
//! The skill's `run(input) -> result<string, string>` maps straight onto
//! [`ToolResult`]: `Ok` → success, `Err` → error. The tool's `approval_level`
//! comes from the **host-trusted** sidecar manifest, never from the guest.
//!
//! wasmtime executes synchronously and is CPU-bound, so the call runs on
//! [`tokio::task::spawn_blocking`] to keep it off the async reactor.

use std::sync::Arc;
use std::{future::Future, pin::Pin};

use serde_json::{Value, json};
use talon_core::{
    approval::ApprovalLevel,
    tools::{Tool, ToolContext, ToolResult},
};
use wasmtime::component::Component;

use crate::host::PluginHost;
use crate::manifest::SkillManifest;

/// A [`Tool`] backed by a compiled skill component plus its sidecar manifest.
pub struct SkillTool {
    host: Arc<PluginHost>,
    component: Component,
    manifest: SkillManifest,
}

impl SkillTool {
    pub fn new(host: Arc<PluginHost>, component: Component, manifest: SkillManifest) -> Self {
        Self {
            host,
            component,
            manifest,
        }
    }
}

/// Build the tool schema from a manifest. Free function so it is testable
/// without a compiled component.
fn skill_schema(manifest: &SkillManifest) -> Value {
    json!({
        "name": manifest.name,
        "description": manifest.description,
        "input_schema": manifest.input_schema,
    })
}

impl Tool for SkillTool {
    fn name(&self) -> &str {
        &self.manifest.name
    }

    fn schema(&self) -> Value {
        skill_schema(&self.manifest)
    }

    /// Sourced from the host-trusted manifest — the guest cannot assert its own
    /// danger level.
    fn approval_level(&self, _args: &Value) -> ApprovalLevel {
        self.manifest.approval_level
    }

    fn execute(
        &self,
        args: Value,
        _ctx: ToolContext,
    ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        let host = self.host.clone();
        let component = self.component.clone();
        let caps = self.manifest.capabilities.clone();
        let name = self.manifest.name.clone();
        Box::pin(async move {
            let input = match serde_json::to_string(&args) {
                Ok(s) => s,
                Err(e) => return ToolResult::err(format!("failed to serialize skill input: {e}")),
            };
            match tokio::task::spawn_blocking(move || host.run(&component, caps, &input)).await {
                Err(e) => ToolResult::err(format!("skill '{name}' task panicked: {e}")),
                Ok(Err(e)) => ToolResult::err(format!("skill '{name}' failed: {e}")),
                Ok(Ok(run)) => match run.output {
                    Ok(s) => ToolResult::ok(s),
                    Err(s) => ToolResult::err(s),
                },
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn schema_carries_name_description_and_input_schema() {
        let manifest = SkillManifest::from_toml(
            r#"
            name = "hello"
            description = "Echo a greeting"
            [input_schema]
            type = "object"
            [input_schema.properties.msg]
            type = "string"
            "#,
        )
        .expect("parse");
        let schema = skill_schema(&manifest);
        assert_eq!(schema["name"], "hello");
        assert_eq!(schema["description"], "Echo a greeting");
        assert_eq!(
            schema["input_schema"]["properties"]["msg"]["type"],
            "string"
        );
    }
}
