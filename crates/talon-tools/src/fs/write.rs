use std::{future::Future, path::Path, pin::Pin};

use serde_json::{Value, json};
use talon_core::{
    approval::ApprovalLevel,
    tools::{Tool, ToolContext, ToolResult},
};

pub struct WriteFileTool;

impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "write_file",
            "description": "Write content to a file atomically (write to temp file, then rename). Creates parent directories if needed.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path":    { "type": "string", "description": "Path to write to." },
                    "content": { "type": "string", "description": "Content to write." }
                },
                "required": ["path", "content"]
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
            let content = match args["content"].as_str() {
                Some(c) => c.to_string(),
                None => return ToolResult::err("missing required argument: content"),
            };

            let dest = Path::new(&path).to_path_buf();

            // Create parent directories if needed.
            if let Some(parent) = dest.parent()
                && let Err(e) = tokio::fs::create_dir_all(parent).await
            {
                return ToolResult::err(format!("cannot create parent dirs for '{path}': {e}"));
            }

            // Atomic write: temp file in same dir → persist (rename) to destination.
            let result = tokio::task::spawn_blocking(move || {
                let parent = dest.parent().unwrap_or(Path::new(".")).to_path_buf();
                let mut tmp =
                    tempfile::NamedTempFile::new_in(&parent).map_err(|e| e.to_string())?;
                use std::io::Write;
                tmp.write_all(content.as_bytes())
                    .map_err(|e| e.to_string())?;
                tmp.persist(&dest).map_err(|e| e.error.to_string())?;
                Ok::<(), String>(())
            })
            .await;

            match result {
                Ok(Ok(())) => ToolResult::ok(format!("wrote '{path}'")),
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
    use tempfile::TempDir;

    #[tokio::test]
    async fn writes_new_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("out.txt");
        let t = WriteFileTool;
        let r = t
            .execute(
                json!({"path": path.to_str().unwrap(), "content": "hello"}),
                ToolContext::default(),
            )
            .await;
        assert!(!r.is_error, "{}", r.content);
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "hello");
    }

    #[tokio::test]
    async fn overwrites_existing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("f.txt");
        tokio::fs::write(&path, b"old").await.unwrap();
        let t = WriteFileTool;
        let r = t
            .execute(
                json!({"path": path.to_str().unwrap(), "content": "new"}),
                ToolContext::default(),
            )
            .await;
        assert!(!r.is_error, "{}", r.content);
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "new");
    }

    #[tokio::test]
    async fn creates_parent_directories() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a/b/c/out.txt");
        let t = WriteFileTool;
        let r = t
            .execute(
                json!({"path": path.to_str().unwrap(), "content": "deep"}),
                ToolContext::default(),
            )
            .await;
        assert!(!r.is_error, "{}", r.content);
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "deep");
    }

    #[tokio::test]
    async fn returns_error_for_missing_path_arg() {
        let t = WriteFileTool;
        let r = t
            .execute(json!({"content": "x"}), ToolContext::default())
            .await;
        assert!(r.is_error);
        assert!(r.content.contains("missing required argument: path"));
    }

    #[tokio::test]
    async fn returns_error_for_missing_content_arg() {
        let t = WriteFileTool;
        let r = t
            .execute(json!({"path": "/tmp/x.txt"}), ToolContext::default())
            .await;
        assert!(r.is_error);
        assert!(r.content.contains("missing required argument: content"));
    }

    #[test]
    fn name_is_write_file() {
        assert_eq!(WriteFileTool.name(), "write_file");
    }

    #[test]
    fn approval_level_is_needs_approval() {
        assert_eq!(
            WriteFileTool.approval_level(&Value::Null),
            ApprovalLevel::NeedsApproval
        );
    }
}
