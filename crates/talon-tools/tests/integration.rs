//! Integration tests for talon-tools (task 3.13).
//!
//! Covers:
//!   - fs roundtrip: write → read → grep → glob
//!   - native mode prepends [NATIVE] tag
//!   - TimeoutWrapper kills a hung process
//!   - Docker blocks `rm -rf /`  (#[ignore] — requires Docker daemon)

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::time::Duration;

use serde_json::json;
use talon_core::tools::{Tool, ToolContext};
use talon_tools::{
    TimeoutWrapper,
    fs::{GlobTool, GrepTool, ReadFileTool, WriteFileTool},
    terminal::{NativeBackend, TerminalTool},
};

// ── fs roundtrip ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_write_then_read_roundtrip() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("hello.txt");
    let path_str = path.to_str().unwrap();

    let written = WriteFileTool
        .execute(
            json!({"path": path_str, "content": "integration test content"}),
            ToolContext::default(),
        )
        .await;
    assert!(!written.is_error, "write failed: {}", written.content);

    let read = ReadFileTool
        .execute(json!({"path": path_str}), ToolContext::default())
        .await;
    assert!(!read.is_error, "read failed: {}", read.content);
    assert_eq!(read.content, "integration test content");
}

#[tokio::test]
async fn fs_grep_finds_written_content() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("source.rs");
    let path_str = path.to_str().unwrap();

    WriteFileTool
        .execute(
            json!({"path": path_str, "content": "fn greet() -> &'static str {\n    \"hello\"\n}"}),
            ToolContext::default(),
        )
        .await;

    let r = GrepTool
        .execute(
            json!({"pattern": "fn greet", "path": dir.path().to_str().unwrap()}),
            ToolContext::default(),
        )
        .await;
    assert!(!r.is_error, "{}", r.content);
    assert!(
        r.content.contains("fn greet"),
        "expected match, got: {}",
        r.content
    );
    assert!(r.content.contains("source.rs"));
}

#[tokio::test]
async fn fs_glob_finds_written_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("main.rs");

    WriteFileTool
        .execute(
            json!({"path": path.to_str().unwrap(), "content": "fn main() {}"}),
            ToolContext::default(),
        )
        .await;

    let r = GlobTool
        .execute(
            json!({"pattern": "**/*.rs", "root": dir.path().to_str().unwrap()}),
            ToolContext::default(),
        )
        .await;
    assert!(!r.is_error, "{}", r.content);
    assert!(
        r.content.contains("main.rs"),
        "expected main.rs in glob output, got: {}",
        r.content
    );
}

#[tokio::test]
async fn fs_write_read_grep_glob_full_chain() {
    let dir = tempfile::TempDir::new().unwrap();
    let file = dir.path().join("chain.txt");
    let file_str = file.to_str().unwrap();
    let dir_str = dir.path().to_str().unwrap();

    WriteFileTool
        .execute(
            json!({"path": file_str, "content": "MARKER_XYZ"}),
            ToolContext::default(),
        )
        .await;

    let read = ReadFileTool
        .execute(json!({"path": file_str}), ToolContext::default())
        .await;
    assert_eq!(read.content, "MARKER_XYZ");

    let grep = GrepTool
        .execute(
            json!({"pattern": "MARKER_XYZ", "path": dir_str}),
            ToolContext::default(),
        )
        .await;
    assert!(grep.content.contains("MARKER_XYZ"));

    let glob = GlobTool
        .execute(
            json!({"pattern": "**/*.txt", "root": dir_str}),
            ToolContext::default(),
        )
        .await;
    assert!(glob.content.contains("chain.txt"));
}

// ── native mode ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn native_tool_output_has_native_tag() {
    let t = TerminalTool::new(NativeBackend);
    let r = t
        .execute(
            json!({"command": "echo native_marker"}),
            ToolContext::default(),
        )
        .await;
    assert!(!r.is_error, "{}", r.content);
    assert!(
        r.content.starts_with("[NATIVE]"),
        "expected [NATIVE] prefix, got: {}",
        r.content
    );
    assert!(r.content.contains("native_marker"));
}

#[tokio::test]
async fn native_tool_error_output_has_native_tag() {
    let t = TerminalTool::new(NativeBackend);
    let r = t
        .execute(json!({"command": "exit 42"}), ToolContext::default())
        .await;
    assert!(r.is_error);
    assert!(
        r.content.starts_with("[NATIVE]"),
        "expected [NATIVE] prefix on error, got: {}",
        r.content
    );
}

// ── timeout kills hung process ────────────────────────────────────────────────

#[tokio::test]
async fn timeout_wrapper_kills_hung_native_command() {
    // sleep 60 would run for a minute; the 100ms timeout must kill it.
    let inner = TerminalTool::new(NativeBackend);
    let wrapped = TimeoutWrapper::new(inner, Duration::from_millis(100));

    let r = wrapped
        .execute(json!({"command": "sleep 60"}), ToolContext::default())
        .await;
    assert!(
        r.is_error,
        "expected timeout error, got success: {}",
        r.content
    );
    assert!(
        r.content.contains("timed out"),
        "expected 'timed out' in error, got: {}",
        r.content
    );
    assert!(
        r.content.contains("run_command"),
        "expected tool name in error, got: {}",
        r.content
    );
}

// ── Docker: rm -rf / is blocked ───────────────────────────────────────────────

/// Requires a live Docker daemon.
/// Run with: cargo nextest run --run-ignored all -E 'test(docker_rm_rf_blocked)'
#[tokio::test]
#[ignore = "requires Docker daemon"]
async fn docker_rm_rf_blocked_by_seccomp() {
    use talon_tools::terminal::{DockerSandbox, SandboxBackend};

    // Use alpine — minimal, widely available.
    let sandbox = DockerSandbox::new("alpine:latest");
    let r = sandbox.execute("rm -rf / 2>&1; echo status:$?").await;

    // Either the container exits non-zero, or the output contains an error message.
    // A seccomp-blocked rm will fail with "Operation not permitted" on the syscalls
    // it needs (unlink, rmdir) even if the process continues.
    // The key invariant: the host filesystem is never touched.
    // We assert the container did not report a clean success of removing /.
    let content = r.content.to_lowercase();
    let clean_removal = !r.is_error
        && !content.contains("operation not permitted")
        && !content.contains("read-only")
        && !content.contains("permission denied");
    assert!(
        !clean_removal,
        "expected rm -rf / to fail inside Docker sandbox, got: {}",
        r.content
    );
}
