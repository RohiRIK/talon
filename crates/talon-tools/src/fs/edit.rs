use std::{future::Future, pin::Pin};

use serde_json::{Value, json};
use talon_core::{
    approval::ApprovalLevel,
    tools::{Tool, ToolContext, ToolResult},
};

pub struct EditFileTool;

impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "edit_file",
            "description": "Replace an exact string in a file. Fails if `old_string` is not found or appears more than once.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path":       { "type": "string", "description": "Path to the file." },
                    "old_string": { "type": "string", "description": "Exact string to replace (must appear exactly once)." },
                    "new_string": { "type": "string", "description": "Replacement string." }
                },
                "required": ["path", "old_string", "new_string"]
            }
        })
    }

    fn approval_level(&self, _args: &Value) -> ApprovalLevel {
        ApprovalLevel::NeedsApproval
    }

    fn execute(
        &self,
        args: Value,
        _ctx: ToolContext,
    ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        Box::pin(async move {
            let path = match args["path"].as_str() {
                Some(p) => p.to_string(),
                None => return ToolResult::err("missing required argument: path"),
            };
            let old = match args["old_string"].as_str() {
                Some(s) => s.to_string(),
                None => return ToolResult::err("missing required argument: old_string"),
            };
            let new = match args["new_string"].as_str() {
                Some(s) => s.to_string(),
                None => return ToolResult::err("missing required argument: new_string"),
            };

            let content = match tokio::fs::read_to_string(&path).await {
                Ok(c) => c,
                Err(e) => return ToolResult::err(format!("cannot read '{path}': {e}")),
            };

            let count = content.matches(old.as_str()).count();
            if count == 0 {
                return ToolResult::err(format!("old_string not found in '{path}'"));
            }
            if count > 1 {
                return ToolResult::err(format!(
                    "old_string appears {count} times in '{path}' — must be unique"
                ));
            }

            let updated = content.replacen(old.as_str(), new.as_str(), 1);

            // Atomic write via temp+rename in the same directory.
            let parent = std::path::Path::new(&path)
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .to_path_buf();
            let path_clone = path.clone();

            let result = tokio::task::spawn_blocking(move || {
                let mut tmp =
                    tempfile::NamedTempFile::new_in(&parent).map_err(|e| e.to_string())?;
                use std::io::Write;
                tmp.write_all(updated.as_bytes())
                    .map_err(|e| e.to_string())?;
                let tmp_path = tmp.path().to_path_buf();
                tmp.persist(&tmp_path).map_err(|e| e.to_string())?;
                std::fs::rename(&tmp_path, &path_clone).map_err(|e| e.to_string())?;
                Ok::<(), String>(())
            })
            .await;

            match result {
                Ok(Ok(())) => ToolResult::ok(format!("edited '{path}'")),
                Ok(Err(e)) => ToolResult::err(format!("write failed for '{path}': {e}")),
                Err(e) => ToolResult::err(format!("internal error: {e}")),
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    async fn make_file(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[tokio::test]
    async fn replaces_unique_string() {
        let f = make_file("hello world").await;
        let path = f.path().to_str().unwrap().to_string();
        let t = EditFileTool;
        let r = t
            .execute(
                json!({"path": path, "old_string": "world", "new_string": "Rust"}),
                ToolContext::default(),
            )
            .await;
        assert!(!r.is_error, "{}", r.content);
        assert_eq!(
            tokio::fs::read_to_string(&path).await.unwrap(),
            "hello Rust"
        );
    }

    #[tokio::test]
    async fn errors_when_old_string_not_found() {
        let f = make_file("hello world").await;
        let path = f.path().to_str().unwrap();
        let t = EditFileTool;
        let r = t
            .execute(
                json!({"path": path, "old_string": "missing", "new_string": "x"}),
                ToolContext::default(),
            )
            .await;
        assert!(r.is_error);
        assert!(r.content.contains("not found"));
    }

    #[tokio::test]
    async fn errors_when_old_string_not_unique() {
        let f = make_file("foo foo").await;
        let path = f.path().to_str().unwrap();
        let t = EditFileTool;
        let r = t
            .execute(
                json!({"path": path, "old_string": "foo", "new_string": "bar"}),
                ToolContext::default(),
            )
            .await;
        assert!(r.is_error);
        assert!(r.content.contains("2 times"));
    }

    #[tokio::test]
    async fn errors_on_missing_file() {
        let t = EditFileTool;
        let r = t
            .execute(
                json!({"path": "/no/such/file.txt", "old_string": "a", "new_string": "b"}),
                ToolContext::default(),
            )
            .await;
        assert!(r.is_error);
        assert!(r.content.contains("cannot read"));
    }

    #[test]
    fn name_is_edit_file() {
        assert_eq!(EditFileTool.name(), "edit_file");
    }

    #[test]
    fn approval_level_is_needs_approval() {
        assert_eq!(
            EditFileTool.approval_level(&Value::Null),
            ApprovalLevel::NeedsApproval
        );
    }

    #[test]
    fn schema_has_required_fields() {
        let s = EditFileTool.schema();
        let req = s["input_schema"]["required"].as_array().unwrap();
        let fields: Vec<&str> = req.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(fields.contains(&"path"));
        assert!(fields.contains(&"old_string"));
        assert!(fields.contains(&"new_string"));
    }
}
