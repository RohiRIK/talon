# Talon

> A fully native Rust AI agent. Single binary. Multi-channel. Persistent cross-project memory.

[![CI](https://github.com/RohiRIK/talon/actions/workflows/ci.yml/badge.svg)](https://github.com/RohiRIK/talon/actions)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

---

## Why Talon

Every other open-source AI agent has at least one of these problems:

| Problem | Who has it |
|---------|-----------|
| Requires Node.js or Python runtime | Claude Code, Hermes, OpenClaw, Aider |
| No persistent memory across sessions | Aider, most CLI agents |
| Single channel (CLI only) | Aider, most CLI agents |
| Memory lives in the cloud | Proprietary agents |
| Slow start due to runtime overhead | Python/Node-based agents |

Talon's answer: **one pre-built binary, zero runtime dependencies, SQLite-backed FTS5 memory that travels with you across projects and channels.**

Start a session on Telegram, continue it in the CLI, search it from the HTTP API — same conversation context, no cloud required.

---

## Features

- **Single binary** — `curl -fsSL talon.sh/install | sh`, nothing else to install
- **Persistent cross-project memory** — SQLite + FTS5 full-text search; every session is queryable
- **Multi-channel** — CLI, TUI, Telegram, Discord, HTTP/SSE from one process
- **Approval membrane** — per-invocation, typed `ApprovalLevel`; dangerous tools cannot run without explicit confirmation
- **Docker-sandboxed terminal** — seccomp-enforced; `rm -rf /` is physically blocked in the sandbox
- **WASM plugins** — hot-reload any language that compiles to `.wasm`; no restart required
- **MCP client** — every Claude Code MCP tool plugs straight in
- **Cron scheduling** — schedule LLM agents to run on a cron expression, stored in SQLite
- **Semantic search** (optional feature) — `fastembed` ONNX embeddings + RRF fusion with FTS5
- **Self-evolving skills** (v2) — DSPy+GEPA Python sidecar improves skill prompts over time

---

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                      TALON RUNTIME                      │
│                                                         │
│  CLI/TUI   Telegram   Discord   HTTP/SSE   Cron         │
│     └──────────┴──────────┴────────┴────────┘           │
│                          │                              │
│                   Gateway Router                        │
│                          │                              │
│                   Agent Session Manager                 │
│                          │                              │
│              ┌───── Core Agent Loop ─────┐              │
│              │  Build Context            │              │
│              │     → LLM Call           │              │
│              │       → Parse            │              │
│              │         → Approval Check │              │
│              │           → Execute Tool │              │
│              │             → Loop       │              │
│              └───────────────────────────┘              │
│          │              │              │                │
│   Tool Registry   Memory Store    LLM Providers         │
│                          │                              │
│                    SQLite (WAL)                         │
│              sessions · messages · FTS5                 │
│              skills · cron · embeddings                 │
└─────────────────────────────────────────────────────────┘
```

**6-crate Cargo workspace:**

| Crate | Role |
|-------|------|
| `talon` | Binary entrypoint, CLI, config |
| `talon-core` | Agent loop, approval membrane, events |
| `talon-llm` | `LlmProvider` trait, OpenAI + Anthropic impls |
| `talon-memory` | SQLite+FTS5, sessions, context assembly |
| `talon-tools` | File, terminal, web, browser, MCP, delegate tools |
| `talon-gateway` | CLI/TUI, Telegram, Discord, HTTP adapters |
| `talon-plugins` | WASM host (wasmtime), skill store, hot-reload |

---

## Status

**Pre-alpha — specification phase complete, implementation starting.**

The full architecture is documented across 65 spec documents in `docs/`. The implementation plan is in [`PLAN.md`](PLAN.md).

| Phase | Description | Status |
|-------|-------------|--------|
| 0 | Foundation — workspace, CI, Docker | Planned |
| 0.5 | Working prototype — validate core loop | Planned |
| 1 | Core agent loop — LLM + tool trait + approval | Planned |
| 2 | Memory — SQLite, FTS5, context assembly | Planned |
| 3 | Tools Tier 1 — file, terminal, sandbox | Planned |
| 4 | Gateway — HTTP, CLI/TUI, Telegram | Planned |
| 5 | Tools Tier 2 — web, browser, MCP | Planned |
| 6 | Plugins & scheduling — WASM, cron, skills | Planned |
| 7 | Advanced — subagents, semantic search, Discord | Planned |

---

## Tech Stack

- **Rust** — `edition = "2024"`, `resolver = "2"`
- **Async** — `tokio` (full), `axum` for HTTP
- **LLM** — `reqwest` + streaming SSE; provider-agnostic trait
- **Memory** — `rusqlite` (bundled + vtab), FTS5, `deadpool-sqlite`
- **TUI** — `ratatui` + `crossterm`
- **Telegram** — `teloxide`
- **WASM** — `wasmtime` (WASI preview2)
- **Embeddings** — `fastembed` (ONNX, feature-flagged)
- **Error handling** — `thiserror` in libs, `anyhow` in binary
- **Testing** — `cargo nextest`
- **CI** — GitHub Actions matrix (linux / macos / windows)

---

## Design Principles

1. **No global state** — all state flows through `Arc<>` refs passed explicitly
2. **Approval membrane** — every tool execution checks typed `ApprovalLevel` per invocation, before running
3. **SQLite as source of truth** — sessions, messages, cron jobs, skills all persisted
4. **`tokio::sync::Mutex` always** — never `std::sync::Mutex` inside async code
5. **`spawn_blocking` for SQLite** — `rusqlite::Connection` is `!Send`; never crosses an `await` point
6. **Stream everything** — LLM responses stream to gateway as they arrive
7. **Three error audiences** — user-facing clean message, developer structured log, LLM-facing tool result
8. **Thin core, thick plugins** — `talon-core` has no opinion on which tools exist

---

## Contributing

Spec documents live in `docs/` — 65 documents covering every aspect of the system. Read [`PLAN.md`](PLAN.md) for the phased build order and [`docs/00_Connections/06_Phase_Build_Guide.md`](docs/00_Connections/06_Phase_Build_Guide.md) for exit gates and checklists.

The critical invariants to know before writing any code:

- `rusqlite::Connection` is `!Send` — use `deadpool-sqlite` pool; never wrap in `tokio::sync::Mutex`
- No `async-trait` crate — native async fn in traits (edition 2024)
- `Arc<dyn Tool>` not `Arc<Box<dyn Tool>>`
- `cargo nextest run` not `cargo test`
- `thiserror` in library crates, `anyhow` in the binary

---

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
