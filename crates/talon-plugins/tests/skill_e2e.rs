//! End-to-end skill tests against a real compiled component.
//!
//! The fixture `tests/fixtures/hello.wasm` is a prebuilt `wasm32-wasip2`
//! component (regenerate with `examples/skills/hello/build.sh`). Loading bytes
//! keeps these tests runnable on any toolchain — CI never compiles wasm.

#![cfg(feature = "wasm")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use talon_core::tools::ToolContext;
use talon_plugins::{PluginHost, SkillStore};

const HELLO_WASM: &[u8] = include_bytes!("fixtures/hello.wasm");
const HELLO_TOML: &str = "name = \"hello\"\n\
     description = \"Greet by name\"\n\
     approval_level = \"safe\"\n\
     capabilities = [\"log\"]\n";

/// Write the hello fixture (wasm + sidecar) into `dir` as `hello.{wasm,toml}`.
fn deploy_hello(dir: &std::path::Path) {
    std::fs::write(dir.join("hello.wasm"), HELLO_WASM).expect("write wasm");
    std::fs::write(dir.join("hello.toml"), HELLO_TOML).expect("write toml");
}

fn host() -> Arc<PluginHost> {
    Arc::new(PluginHost::new().expect("host"))
}

#[test]
fn loads_and_runs_a_real_skill() {
    let dir = tempfile::tempdir().expect("tmp");
    deploy_hello(dir.path());

    let store = SkillStore::new(host(), dir.path().to_path_buf());
    assert_eq!(store.names(), vec!["hello".to_string()]);

    let run = store.run("hello", r#"{"name":"Talon"}"#).expect("run");
    assert_eq!(run.output, Ok("Hello, Talon!".to_string()));
    assert!(
        run.logs.iter().any(|l| l.contains("Talon")),
        "host log capability captured a line: {:?}",
        run.logs
    );
}

#[test]
fn missing_name_defaults_to_world() {
    let dir = tempfile::tempdir().expect("tmp");
    deploy_hello(dir.path());
    let store = SkillStore::new(host(), dir.path().to_path_buf());

    let run = store.run("hello", "{}").expect("run");
    assert_eq!(run.output, Ok("Hello, world!".to_string()));
}

#[tokio::test]
async fn skill_exposes_as_tool() {
    let dir = tempfile::tempdir().expect("tmp");
    deploy_hello(dir.path());
    let store = SkillStore::new(host(), dir.path().to_path_buf());

    let tools = store.tools();
    assert_eq!(tools.len(), 1);
    let tool = &tools[0];
    assert_eq!(tool.name(), "hello");
    assert_eq!(
        tool.approval_level(&serde_json::json!({})),
        talon_core::approval::ApprovalLevel::Safe
    );

    let result = tool
        .execute(
            serde_json::json!({ "name": "Tool" }),
            ToolContext::default(),
        )
        .await;
    assert!(!result.is_error, "got: {}", result.content);
    assert_eq!(result.content, "Hello, Tool!");
}

#[test]
fn hot_reload_picks_up_dropped_skill_within_2s() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut store = SkillStore::new(host(), dir.path().to_path_buf());
    assert!(store.names().is_empty(), "starts empty");
    store.watch().expect("watch");

    let start = Instant::now();
    deploy_hello(dir.path());

    // Poll until the watcher re-scans and the skill appears.
    loop {
        if store.names() == vec!["hello".to_string()] {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "skill did not hot-reload within 2s"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // And it actually runs after the live reload.
    let run = store.run("hello", r#"{"name":"Reload"}"#).expect("run");
    assert_eq!(run.output, Ok("Hello, Reload!".to_string()));
}
