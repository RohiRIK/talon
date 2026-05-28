use std::{future::Future, pin::Pin};

use serde_json::{Value, json};
use talon_core::{
    approval::ApprovalLevel,
    tools::{Tool, ToolContext, ToolResult},
};

const MAX_BYTES: u64 = 10 * 1024 * 1024; // 10 MB

pub struct ReadFileTool;

impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "read_file",
            "description": "Read the contents of a file. Fails if the file exceeds 10 MB.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute or relative path to the file." }
                },
                "required": ["path"]
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
            let path = match args["path"].as_str() {
                Some(p) => p.to_string(),
                None => return ToolResult::err("missing required argument: path"),
            };

            let metadata = match tokio::fs::metadata(&path).await {
                Ok(m) => m,
                Err(e) => return ToolResult::err(format!("cannot read '{path}': {e}")),
            };

            if metadata.len() > MAX_BYTES {
                return ToolResult::err(format!(
                    "file '{path}' is {} bytes — exceeds 10 MB limit",
                    metadata.len()
                ));
            }

            match tokio::fs::read_to_string(&path).await {
                Ok(content) => ToolResult::ok(content),
                Err(e) => ToolResult::err(format!("cannot read '{path}': {e}")),
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

    #[tokio::test]
    async fn reads_existing_file() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"hello world").unwrap();
        let path = f.path().to_str().unwrap().to_string();

        let t = ReadFileTool;
        let r = t
            .execute(json!({"path": path}), ToolContext::default())
            .await;
        assert!(!r.is_error);
        assert_eq!(r.content, "hello world");
    }

    #[tokio::test]
    async fn returns_error_for_missing_file() {
        let t = ReadFileTool;
        let r = t
            .execute(
                json!({"path": "/nonexistent/path/xyz.txt"}),
                ToolContext::default(),
            )
            .await;
        assert!(r.is_error);
        assert!(r.content.contains("cannot read"));
    }

    #[tokio::test]
    async fn returns_error_for_missing_path_arg() {
        let t = ReadFileTool;
        let r = t.execute(json!({}), ToolContext::default()).await;
        assert!(r.is_error);
        assert!(r.content.contains("missing required argument"));
    }

    #[test]
    fn name_is_read_file() {
        assert_eq!(ReadFileTool.name(), "read_file");
    }

    #[test]
    fn approval_level_is_safe() {
        assert_eq!(
            ReadFileTool.approval_level(&Value::Null),
            ApprovalLevel::Safe
        );
    }

    #[test]
    fn schema_has_path_property() {
        let s = ReadFileTool.schema();
        assert_eq!(s["name"], "read_file");
        assert!(s["input_schema"]["properties"]["path"].is_object());
    }
}
