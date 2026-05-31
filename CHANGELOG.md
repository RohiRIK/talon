# Changelog

All notable changes to the Talon project will be documented in this file.

## [Unreleased]

### Added — Phase 2.5: Talon LTM (SQLite + sqlite-vec)
- **`talon-ltm` memory layer** — long-term memory implemented natively in Rust over a single SQLite database (ADR 0008 — supersedes the earlier LanceDB plan; `sqlite-vec` for vectors + FTS5 for keyword + RRF fusion in Rust, no LanceDB/Redis):
  - `sqlite-vec` extension wired into the `deadpool-sqlite` pool (2.5.1)
  - `LtmStore` over SQLite — `memories` table + `memories_fts` (FTS5, porter stemming) + `vec_memories` (2.5.2)
  - Typed `MemoryCategory` enum + tags on the LTM model (2.5.3)
  - Token-budgeted `WorkingMemory` with rolling summary (2.5.4)
  - LLM-powered `FactExtractor` with a Markdown extraction prompt (2.5.5)
  - Semantic deduplication of memories (2.5.6)
  - `Promoter` — promotes high-importance session facts to LTM (2.5.7)
  - `HybridSearch` — hybrid FTS5 + vector retrieval via Reciprocal Rank Fusion, k=60 (2.5.8)
  - `SemanticCache` — semantic LLM response cache (2.5.9)
  - `DecayEngine` — time-based memory decay (2.5.10)
  - `ContextBuilder` folds overflow turns through `WorkingMemory` (2.5.11)
  - End-to-end LTM integration tests (2.5.12)
- **CLI:** `talon memory` (stats) and `talon cache` (stats/clear) subcommands (2.5.13)
- **Agent runtime wiring (2.5.14)** — LTM is now live in the agent loop: FTS5 recall at the start of each turn (injected into the system prompt) and automatic LLM fact-extraction → promotion at turn end. Recall queries are sanitized via `fts5_or_query` to keep raw user text from breaking `MATCH`. Live cross-session recall verified end-to-end through the key-less `github-copilot` provider.

### Added
- **Redis Iris integration docs** (`docs/09_Redis_Iris/`) — 5 new documents exploring Redis Iris as Talon's context engine:
  - `66_Redis_Iris_Overview.md` — Strategic fit analysis, three integration options (SQLite-only / Redis-only / Hybrid), recommendation for Option C (feature-flagged)
  - `67_Iris_Two_Tier_Memory.md` — Two-tier memory architecture (working + long-term), auto-summarization, LLM fact extraction, semantic deduplication, memory promotion
  - `68_Iris_Technical_Integration.md` — Rust integration strategies (sidecar vs native vs MCP), `redis` crate usage, Redis data model, feature flag design, performance benchmarks
  - `69_Iris_Philosophy.md` — Design principles ("context is all you need"), philosophical tension with single-binary story, what to adopt vs skip
  - `70_Iris_Semantic_Cache.md` — LangCache-inspired semantic response caching, cost optimization estimates, cache invalidation strategy
- **Phase 2.5 in PLAN.md** — Redis Iris Memory Layer: 12 tasks covering two-tier memory, fact extraction, semantic dedup, hybrid search, semantic cache, and optional Redis backend
- **roadmap.md** — Chronological implementation roadmap with dependency graph, week-by-week timeline, critical path analysis (~12 weeks to v1.0), priority stack for time-constrained builds, and risk register
- Updated `00_Master_Index.md` — Added section 09_Redis_Iris (5 docs), total doc count 65→70, completed count 9→14
- Updated Final Acceptance Criteria in PLAN.md — Added Iris memory, semantic cache, and Redis backend gates
- **Brain candidates research** (`docs/09_Redis_Iris/`) — 5 new documents evaluating memory/context engine candidates for Talon:
  - `71_Brain_Candidates_Overview.md` — Comparison matrix of 6 candidates (vector search, FTS, decay, auto-extraction, maturity)
  - `72_Claude_LTM_Analysis.md` — claude-ltm-plugin deep dive ★★★★★: categories, importance scoring, decay, typed memory graphs, auto-extraction, FTS5-first search — recommended as Talon's design blueprint
  - `73_LanceDB_Analysis.md` — LanceDB embedded vector+FTS DB ★★★★☆: SQLite-like but with native vectors, potential storage engine replacement
  - `74_Mem0_Rust_Analysis.md` — mem0-rust agent memory layer ★★★☆☆: multi-backend, auto-extraction, thin memory model
  - `75_Qdrant_Rig_Swiftide_Analysis.md` — Ecosystem players ★★–★★★: Qdrant (mature but needs server), Rig (clean traits), Swiftide (pipeline patterns)
- Updated `00_Master_Index.md` — Added docs 71–75, total doc count 70→75, completed count 14→19
- Updated graphify knowledge graph — 3026→3130 nodes, 2892→2989 edges, 206→214 communities
- **Emerging recommendation**: claude-ltm memory model as design blueprint + LanceDB as storage engine (feature-flagged: `sqlite-memory` default vs `lance-memory`)
- **Honker reactive layer** (`docs/09_Redis_Iris/76_Honker_Reactive_Layer.md`) — SQLite NOTIFY/LISTEN + durable queues + streams + scheduler as Talon's nervous system. Pairs with talon-ltm (own Rust reimplementation of claude-ltm blueprint) + LanceDB. Graph layer optional/later.
- Updated `00_Master_Index.md` — Added doc 76, completed count 14→15
- **Architecture decision:** claude-ltm is a **blueprint to reimplement** as `talon-ltm` in Rust, NOT a direct dependency. Honker adds reactive plumbing on top. Graph is optional.
- **TUI research & technology selection** (`docs/10_TUI/`) — 3 new documents:
  - `77_TUI_Landscape_Overview.md` — Comprehensive comparison of TUI frameworks (Ratatui, Ink, Textual, Bubbletea, Cursive) + how AI CLIs (Claude Code, OpenCode, Aider, Amazon Q) build their interfaces
  - `78_TUI_Technology_Selection.md` — Decision: Ratatui + Crossterm + MVU architecture. Component design (ChatView, InputBar, ToolPanel, StatusBar), async integration, adaptive layout, essential crates list
  - `79_Terminal_Rendering_Capabilities.md` — Image protocols (Kitty/Sixel/iTerm2), streaming markdown rendering, OSC 8 clickable links, accessibility (`NO_COLOR`, `--accessible`), multiplexer awareness, web hybrid (xterm.js), diff rendering
- Updated `00_Master_Index.md` — Added section 10_TUI (3 docs), completed count 15→18
- **Architecture decision: LanceDB from day one** — dropped `sqlite-memory` vs `lance-memory` feature flag. LanceDB is the sole memory backend (FTS + vectors + hybrid search). SQLite remains for non-memory concerns (sessions, config, Honker coordination). One backend, one path, no throwaway code.
- Updated docs 72, 73, 76 to reflect unified architecture: talon-ltm (claude-ltm blueprint) + LanceDB (storage) + Honker (reactive layer). Graph optional/later.
