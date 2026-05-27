# Changelog

All notable changes to the Talon project will be documented in this file.

## [Unreleased]

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
