# Build System: Cargo Workspace

> **Status:** ✅ Complete
> **Category:** DevOps
> **Last corrected:** dogfood pass 4

---

## 1. Workspace Structure

```
talon/
├── Cargo.toml              # workspace root
├── Cargo.lock
├── talon/                  # binary crate
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       └── cli.rs
└── crates/
    ├── talon-core/        # agent loop, tools, approval
    │   ├── Cargo.toml
    │   └── src/
    ├── talon-llm/         # LLM providers (Anthropic, OpenAI-compat, Ollama)
    │   ├── Cargo.toml
    │   └── src/
    ├── talon-memory/      # SQLite, FTS5, sessions, memory files
    │   ├── Cargo.toml
    │   └── src/
    ├── talon-gateway/     # Telegram, Discord, HTTP, CLI/TUI
    │   ├── Cargo.toml
    │   └── src/
    ├── talon-plugins/     # WASM runtime, plugin loader
    │   ├── Cargo.toml
    │   └── src/
    └── talon-tools/       # Built-in tool implementations
        ├── Cargo.toml
        └── src/
```

---

## 2. Root Cargo.toml

```toml
[workspace]
resolver = "2"
members = [
    "talon",              # binary
    "crates/talon-core",
    "crates/talon-llm",
    "crates/talon-memory",
    "crates/talon-gateway",
    "crates/talon-plugins",
    "crates/talon-tools",
]

[workspace.package]
edition = "2024"
version = "0.1.0"
authors = ["Talon Contributors"]
license = "MIT"
repository = "https://github.com/RohiRIK/talon"

[workspace.dependencies]
# Async runtime
tokio = { version = "1", features = ["full"] }
tokio-stream = "0.1"
futures = "0.3"
async-trait = "0.1"

# HTTP
reqwest = { version = "0.12", features = ["json", "stream", "multipart"] }
axum = { version = "0.7", features = ["macros"] }
tower = "0.4"
tower-http = { version = "0.5", features = ["trace", "timeout"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"

# Error handling
thiserror = "1"
anyhow = "1"

# Logging/Tracing
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

# Database
rusqlite = { version = "0.31", features = ["bundled", "vtab", "load_extension"] }
r2d2 = "0.8"
r2d2_sqlite = "0.24"

# Config
config = "0.14"
toml = "0.8"

# TUI
ratatui = "0.27"
crossterm = "0.27"

# Telegram
teloxide = { version = "0.13", features = ["macros"] }

# Discord
serenity = { version = "0.12", features = ["client", "gateway", "model"] }

# UUID
uuid = { version = "1", features = ["v4"] }

# Time
chrono = { version = "0.4", features = ["serde"] }

# Regex
regex = "1"

# WASM
wasmtime = { version = "21", optional = true }

# Embeddings (optional)
fastembed = { version = "3", optional = true }

[profile.release]
lto = "thin"
codegen-units = 1
strip = "debuginfo"
opt-level = 3

[profile.dev]
opt-level = 1    # Slightly optimized debug builds

[profile.ci]
inherits = "dev"
debug = false    # Faster CI builds
```

---

## 3. Crate Dependency Graph

```
main (binary)
├── talon-core
│   ├── talon-llm
│   ├── talon-memory
│   └── talon-tools
├── talon-gateway
│   └── talon-core
└── talon-plugins
    └── talon-core
```

No circular dependencies. `talon-core` is the hub — it knows about
tools, memory, and LLM but not about gateways.

---

## 4. Feature Flags

```toml
# talon-memory/Cargo.toml
[features]
default = []
semantic-search = ["dep:fastembed"]
sqlite-vec = []    # enable vector extension loading

# talon-plugins/Cargo.toml
[features]
default = []
wasm = ["dep:wasmtime"]
```

Install with full features:
```bash
cargo build --release \
  --features talon-memory/semantic-search \
  --features talon-plugins/wasm
```

---

## 5. Build Commands

```bash
# Development
cargo build

# Release
cargo build --release

# Run with hot reload (cargo-watch)
cargo watch -x run

# Run tests for all crates
cargo nextest run --workspace

# Check without building
cargo check --workspace

# Clippy with strict lints
cargo clippy --workspace -- -D warnings

# Generate docs
cargo doc --workspace --open

# Security audit
cargo audit
```

---

## 6. Binary Size Optimization

Release binary size targets: <20MB (most common case), <40MB with WASM.

```bash
# Check binary size
cargo bloat --release --crates

# Strip debug symbols
strip target/release/talon

# UPX compression (optional, adds ~300ms startup)
upx --best target/release/talon
```
---

## Related Documents

### Depends On
- [Cargo Workspace Design](../02_Architecture/12_Workspace_And_Crate_Structure.md)

### See Also
- [CI/CD Pipeline](62_CI_CD_Pipeline.md)
- [Release & Distribution](65_Release_And_Distribution.md)

