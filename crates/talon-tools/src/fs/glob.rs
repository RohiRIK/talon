use std::{future::Future, pin::Pin};

use globset::{Glob, GlobSetBuilder};
use serde_json::{Value, json};
use talon_core::{
    approval::ApprovalLevel,
    tools::{Tool, ToolContext, ToolResult},
};
use walkdir::WalkDir;

pub struct GlobTool;

impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "glob",
            "description": "Find files matching a glob pattern. Returns newline-separated list of matching paths.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Glob pattern, e.g. 'src/**/*.rs'" },
                    "root":    { "type": "string", "description": "Directory to search from (default: current dir)." }
                },
                "required": ["pattern"]
            }
        })
    }

    fn approval_level(&self, _args: &Value) -> ApprovalLevel {
        ApprovalLevel::Safe
    }

    fn execute(
        &self,
        args: Value,
        _ctx: ToolContext,
    ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        Box::pin(async move {
            let pattern = match args["pattern"].as_str() {
                Some(p) => p.to_string(),
                None => return ToolResult::err("missing required argument: pattern"),
            };
            let root = args["root"].as_str().unwrap_or(".").to_string();

            let result = tokio::task::spawn_blocking(move || {
                let glob = Glob::new(&pattern).map_err(|e| e.to_string())?;
                let mut builder = GlobSetBuilder::new();
                builder.add(glob);
                let set = builder.build().map_err(|e| e.to_string())?;

                let mut matches: Vec<String> = Vec::new();
                for entry in WalkDir::new(&root).follow_links(false) {
                    let entry = entry.map_err(|e| e.to_string())?;
                    if entry.file_type().is_file() {
                        // Match against path relative to root for glob semantics.
                        let rel = entry.path().strip_prefix(&root).unwrap_or(entry.path());
                        if set.is_match(rel) {
                            matches.push(entry.path().display().to_string());
                        }
                    }
                }
                matches.sort();
                Ok::<Vec<String>, String>(matches)
            })
            .await;

            match result {
                Ok(Ok(paths)) if paths.is_empty() => ToolResult::ok("no matches"),
                Ok(Ok(paths)) => ToolResult::ok(paths.join("\n")),
                Ok(Err(e)) => ToolResult::err(format!("glob error: {e}")),
                Err(e) => ToolResult::err(format!("internal error: {e}")),
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn make_tree() -> TempDir {
        let dir = TempDir::new().unwrap();
        tokio::fs::write(dir.path().join("a.rs"), b"")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("b.txt"), b"")
            .await
            .unwrap();
        tokio::fs::create_dir(dir.path().join("sub")).await.unwrap();
        tokio::fs::write(dir.path().join("sub/c.rs"), b"")
            .await
            .unwrap();
        dir
    }

    #[tokio::test]
    async fn finds_rs_files_with_glob() {
        let dir = make_tree().await;
        let root = dir.path().to_str().unwrap();
        let t = GlobTool;
        let r = t
            .execute(
                json!({"pattern": "**/*.rs", "root": root}),
                ToolContext::default(),
            )
            .await;
        assert!(!r.is_error, "{}", r.content);
        assert!(r.content.contains("a.rs"));
        assert!(r.content.contains("c.rs"));
        assert!(!r.content.contains("b.txt"));
    }

    #[tokio::test]
    async fn returns_no_matches_when_none() {
        let dir = make_tree().await;
        let root = dir.path().to_str().unwrap();
        let t = GlobTool;
        let r = t
            .execute(
                json!({"pattern": "**/*.go", "root": root}),
                ToolContext::default(),
            )
            .await;
        assert!(!r.is_error);
        assert_eq!(r.content, "no matches");
    }

    #[tokio::test]
    async fn returns_error_for_missing_pattern() {
        let t = GlobTool;
        let r = t.execute(json!({}), ToolContext::default()).await;
        assert!(r.is_error);
        assert!(r.content.contains("missing required argument: pattern"));
    }

    #[test]
    fn name_is_glob() {
        assert_eq!(GlobTool.name(), "glob");
    }

    #[test]
    fn approval_level_is_safe() {
        assert_eq!(GlobTool.approval_level(&Value::Null), ApprovalLevel::Safe);
    }
}
