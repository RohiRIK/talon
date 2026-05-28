use std::{future::Future, pin::Pin};

use regex::Regex;
use serde_json::{Value, json};
use talon_core::{
    approval::ApprovalLevel,
    tools::{Tool, ToolContext, ToolResult},
};
use walkdir::WalkDir;

pub struct GrepTool;

impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "grep",
            "description": "Search for a regex pattern in files. Returns matching lines with file:line format.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "pattern":    { "type": "string", "description": "Regex pattern to search for." },
                    "path":       { "type": "string", "description": "File or directory to search (default: current dir)." },
                    "file_glob":  { "type": "string", "description": "Optional glob filter for filenames, e.g. '*.rs'." }
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
            let search_path = args["path"].as_str().unwrap_or(".").to_string();
            let file_glob = args["file_glob"].as_str().map(str::to_string);

            let result = tokio::task::spawn_blocking(move || {
                let re = Regex::new(&pattern).map_err(|e| e.to_string())?;

                // Build optional glob matcher for filename filter.
                let glob_set = if let Some(ref g) = file_glob {
                    let glob = globset::Glob::new(g).map_err(|e| e.to_string())?;
                    let mut b = globset::GlobSetBuilder::new();
                    b.add(glob);
                    Some(b.build().map_err(|e| e.to_string())?)
                } else {
                    None
                };

                let mut output = Vec::new();
                for entry in WalkDir::new(&search_path).follow_links(false) {
                    let entry = entry.map_err(|e| e.to_string())?;
                    if !entry.file_type().is_file() {
                        continue;
                    }

                    // Apply file_glob filter on filename only.
                    if let Some(ref gs) = glob_set {
                        let fname = entry.file_name().to_string_lossy();
                        if !gs.is_match(fname.as_ref()) {
                            continue;
                        }
                    }

                    // Skip files > 1 MB to avoid blowing memory.
                    if entry.metadata().map(|m| m.len()).unwrap_or(0) > 1_048_576 {
                        continue;
                    }

                    let content = match std::fs::read_to_string(entry.path()) {
                        Ok(c) => c,
                        Err(_) => continue, // skip binary / unreadable files
                    };

                    for (i, line) in content.lines().enumerate() {
                        if re.is_match(line) {
                            output.push(format!("{}:{}: {}", entry.path().display(), i + 1, line));
                        }
                    }
                }

                Ok::<Vec<String>, String>(output)
            })
            .await;

            match result {
                Ok(Ok(lines)) if lines.is_empty() => ToolResult::ok("no matches"),
                Ok(Ok(lines)) => ToolResult::ok(lines.join("\n")),
                Ok(Err(e)) => ToolResult::err(format!("grep error: {e}")),
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
        tokio::fs::write(dir.path().join("a.rs"), b"fn main() {}\nlet x = 1;")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("b.txt"), b"hello world\nfoo bar")
            .await
            .unwrap();
        dir
    }

    #[tokio::test]
    async fn finds_matching_lines() {
        let dir = make_tree().await;
        let t = GrepTool;
        let r = t
            .execute(
                json!({"pattern": "fn main", "path": dir.path().to_str().unwrap()}),
                ToolContext::default(),
            )
            .await;
        assert!(!r.is_error, "{}", r.content);
        assert!(r.content.contains("fn main"));
        assert!(r.content.contains("a.rs"));
    }

    #[tokio::test]
    async fn file_glob_filters_by_extension() {
        let dir = make_tree().await;
        let t = GrepTool;
        let r = t.execute(
            json!({"pattern": "hello", "path": dir.path().to_str().unwrap(), "file_glob": "*.rs"}),
            ToolContext::default(),
        ).await;
        // "hello" is in b.txt, not in .rs files
        assert_eq!(r.content, "no matches");
    }

    #[tokio::test]
    async fn returns_no_matches_when_none() {
        let dir = make_tree().await;
        let t = GrepTool;
        let r = t
            .execute(
                json!({"pattern": "zzznomatch", "path": dir.path().to_str().unwrap()}),
                ToolContext::default(),
            )
            .await;
        assert!(!r.is_error);
        assert_eq!(r.content, "no matches");
    }

    #[tokio::test]
    async fn returns_error_for_missing_pattern() {
        let t = GrepTool;
        let r = t.execute(json!({}), ToolContext::default()).await;
        assert!(r.is_error);
    }

    #[tokio::test]
    async fn returns_error_for_invalid_regex() {
        let t = GrepTool;
        let r = t
            .execute(json!({"pattern": "[invalid("}), ToolContext::default())
            .await;
        assert!(r.is_error);
        assert!(r.content.contains("grep error"));
    }

    #[test]
    fn name_is_grep() {
        assert_eq!(GrepTool.name(), "grep");
    }

    #[test]
    fn approval_level_is_safe() {
        assert_eq!(GrepTool.approval_level(&Value::Null), ApprovalLevel::Safe);
    }
}
