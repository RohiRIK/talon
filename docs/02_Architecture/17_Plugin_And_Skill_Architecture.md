# Plugin & Skill Architecture

> **Last corrected:** dogfood pass 2
>
> **Status:** ✅ Complete
> **Category:** Architecture

---

## 1. Two Extension Mechanisms

Talon has two ways to extend its behavior:

| Mechanism | Purpose | Who writes it | Runtime |
|-----------|---------|---------------|---------|
| **Skill** | Procedural memory — markdown instructions | User or agent | No runtime overhead |
| **Plugin** | New tools or integrations | Developer | WASM or native Rust |

Skills are cheap: they're injected text. Plugins are powerful: they add new executable capabilities.

---

## 2. Skill Architecture (Recap)

See doc `57_Skill_File_Management.md` for full detail.

The short version:
```
~/.talon/profiles/<name>/skills/
├── github-pr-workflow.md       # standalone skill
├── devops/
│   ├── homelab-docker-swarm.md
│   └── supabase-local.md
└── software-development/
    ├── spec.md
    └── dev-workflow.md
```

Skills are discovered at startup, listed in context as `<available_skills>`,
and loaded in full only when the LLM calls `skill_view(name)`.

---

## 3. Plugin Architecture

Plugins extend Talon with new `Tool` implementations. Two deployment modes:

### 3.1 Native Rust Plugin (Compiled)
A Rust crate that implements the `Tool` trait and is compiled into Talon:

```rust
// Hypothetical: a Spotify tool plugin
// talon-plugins/spotify/src/lib.rs

pub struct SpotifyTool {
    client: SpotifyClient,
}

#[async_trait]
impl Tool for SpotifyTool {
    fn name(&self) -> &str { "spotify" }
    fn description(&self) -> &str {
        "Control Spotify: play, pause, search, queue, manage playlists"
    }
    fn schema(&self) -> serde_json::Value { /* ... */ serde_json::Value::Null }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult {
        let action: SpotifyAction = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(e.to_string()),
        };
        match action {
            SpotifyAction::Play { query } => match self.client.play(&query).await {
                Ok(_) => {}
                Err(e) => return ToolResult::error(e.to_string()),
            },
            SpotifyAction::Pause => match self.client.pause().await {
                Ok(_) => {}
                Err(e) => return ToolResult::error(e.to_string()),
            },
        }
        ToolResult::success("Done")
    }
}
```

### 3.2 WASM Plugin (Dynamic)
A plugin compiled to WASM and loaded at runtime via `wasmtime`:

```rust
// talon-plugins/src/wasm_runtime.rs

pub struct WasmPlugin {
    engine: wasmtime::Engine,
    module: wasmtime::Module,
    name: String,
    description: String,
}

#[async_trait]
impl Tool for WasmPlugin {
    fn name(&self) -> &str { &self.name }           // &str tied to self lifetime, not &'static str
    fn description(&self) -> &str { &self.description }
    fn schema(&self) -> serde_json::Value { /* loaded from WASM exports */ serde_json::Value::Null }

    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        // Run WASM module in isolated sandbox
        let input = match serde_json::to_vec(&args) {
            Ok(b) => b,
            Err(e) => return ToolResult::error(e.to_string()),
        };
        let output = tokio::task::spawn_blocking({
            let module = self.module.clone();
            let engine = self.engine.clone();
            move || {
                let mut store = wasmtime::Store::new(&engine, ());
                let instance = wasmtime::Instance::new(&mut store, &module, &[])?;
                let run_fn = instance.get_typed_func::<(i32, i32), (i32, i32)>(
                    &mut store, "run"
                )?;
                // Pass input bytes, get output bytes
                // (Full implementation uses WASM memory management)
                Ok::<Vec<u8>, anyhow::Error>(vec![])
            }
        }).await;

        let bytes = match output {
            Ok(Ok(b)) => b,
            Ok(Err(e)) => return ToolResult::error(e.to_string()),
            Err(e) => return ToolResult::error(e.to_string()),
        };

        serde_json::from_slice(&bytes)
            .unwrap_or_else(|e| ToolResult::error(e.to_string()))
    }
}
```

WASM plugins are sandboxed: no filesystem access, no network unless
explicitly granted via WASI capabilities.

---

## 4. Plugin Discovery

```
~/.talon/plugins/
├── spotify.wasm          # WASM plugin
├── homeassistant.wasm    # WASM plugin
└── custom-crm/           # Native plugin source (compiled on install)
    ├── Cargo.toml
    └── src/lib.rs
```

At startup:
```rust
pub async fn load_plugins(plugin_dir: &Path) -> Result<Vec<Box<dyn Tool>>> {
    let mut tools = vec![];

    for entry in std::fs::read_dir(plugin_dir)?.flatten() {
        let path = entry.path();
        match path.extension().and_then(|e| e.to_str()) {
            Some("wasm") => {
                let plugin = WasmPlugin::load(&path).await?;
                tools.push(Box::new(plugin) as Box<dyn Tool>);
            }
            _ => {}
        }
    }

    Ok(tools)
}
```

---

## 5. Plugin Configuration

```toml
# ~/.talon/profiles/default/config.toml

[[plugins]]
name = "spotify"
wasm = "~/.talon/plugins/spotify.wasm"
enabled = true

[plugins.spotify.config]
client_id = "${env:SPOTIFY_CLIENT_ID}"
client_secret = "${env:SPOTIFY_CLIENT_SECRET}"

[[plugins]]
name = "homeassistant"
wasm = "~/.talon/plugins/homeassistant.wasm"
enabled = true

[plugins.homeassistant.config]
base_url = "http://homeassistant.local:8123"
token = "${env:HA_TOKEN}"
```

---

## 6. Tool Registry Composition

On startup, Talon assembles the tool registry from multiple sources:

```rust
pub fn build_tool_registry(config: &Config) -> Arc<ToolRegistry> {
    let mut registry = ToolRegistry::new();

    // 1. Built-in tools (always present)
    registry.register(Box::new(TerminalTool::new(&config.tools.terminal)));
    registry.register(Box::new(ReadFileTool));
    registry.register(Box::new(WriteFileTool));
    registry.register(Box::new(WebSearchTool::new(&config.tools.web_search)));
    registry.register(Box::new(WebExtractTool));
    registry.register(Box::new(MemoryTool::new()));
    registry.register(Box::new(SkillManageTool));
    registry.register(Box::new(SkillViewTool));
    registry.register(Box::new(SkillsListTool));
    // ... all built-in tools

    // 2. Enabled plugins (WASM or native)
    for plugin in load_plugins(&config.plugins_dir).await.unwrap_or_default() {
        registry.register(plugin);
    }

    // 3. MCP tools (if MCP servers configured)
    for (name, mcp_tool) in load_mcp_tools(&config.mcp).await.unwrap_or_default() {
        registry.register(Box::new(mcp_tool));
    }

    Arc::new(registry)
}
```
---

## Related Documents

### Depends On
- [Tool System Architecture](16_Tool_System_Architecture.md)
- [Cargo Workspace Design](12_Workspace_And_Crate_Structure.md)

### Used By
- [Skill Store](../07_Memory_System/57_Skill_Store.md)
- [Self-Evolution Loop](../04_Core_Features/39_Self_Evolution_Loop.md)
- [Skill System](../04_Core_Features/34a_Skill_System.md)

### See Also
- [Approval Membrane](17a_Approval_Membrane.md)
- [Security Model](20_Security_Model.md)
- [Profile Isolation](../04_Core_Features/40_Profile_Isolation.md)

