# ADR 0008 — SQLite + sqlite-vec as the Memory Backend (supersedes 0005)

**Status:** Accepted
**Date:** 2026-05-30
**Supersedes:** [ADR 0005 — LanceDB as Memory Storage Backend](0005-lancedb-memory-backend.md)

## Context

ADR 0005 chose LanceDB for vector memory, keeping SQLite for operational data — two
stores. In practice this fights Talon's core identity ("single binary, **SQLite-backed**
FTS5 memory that travels with you across projects") and creates a dual-write problem
(a fact lives in SQLite, its vector in Lance — they can diverge). LanceDB also pulls the
heavy Arrow dependency tree (the source of the M.3 CVE churn) and ships a pre-1.0 Rust API
we have already been forced to bump (0.9 → 0.29).

ADR 0005's case against `sqlite-vec` was weak: it dinged "no built-in FTS" and "manual
hybrid", but Talon **already uses FTS5**, and Reciprocal Rank Fusion is ~20 lines of Rust.

## Decision

**One SQLite database holds everything.** Vectors live in the same `~/.talon/talon.db`
alongside messages, FTS5 indexes, and config:

- **`sqlite-vec`** (C extension, Apache-2.0, ~7.6k★, Mozilla-backed) — vector storage +
  brute-force KNN, statically linked into the SQLite we already bundle via `rusqlite`.
- **FTS5** — keyword search (already in use).
- **Hybrid retrieval** — FTS5 BM25 ⊕ sqlite-vec KNN, fused with RRF in Rust.
- **Embeddings** — `fastembed` (all-MiniLM-L6-v2, ONNX) behind a feature flag (unchanged
  from the original plan).

LanceDB and `arrow-array` are dropped.

**Honker** (`honker-core`, a SQLite NOTIFY/LISTEN + queue + cron extension) remains the
optional reactive layer, deferred to Phase 6 — where it can replace the hand-rolled
CronScheduler and drive reactive memory maintenance. Because it is also SQLite-based, it
composes cleanly with this decision (same DB, same transaction).

## Why sqlite-vec over LanceDB

| Criterion | LanceDB (0005) | **SQLite + sqlite-vec + FTS5** |
|-----------|----------------|--------------------------------|
| Storage | Separate Lance/Arrow files + SQLite | One `.db` file |
| Consistency | dual-write (fact vs vector can diverge) | fact + embedding + FTS index in one transaction |
| Deps / footprint | heavy Arrow tree | tiny C ext into bundled SQLite |
| Hybrid + RRF | built-in | FTS5 + KNN, RRF ~20 lines |
| ANN index | yes (matters at millions) | brute-force KNN (sub-ms at personal scale) |
| Stability | pre-1.0, churny | SQLite rock-solid; vec surface tiny |
| Honker fits | no (Honker is SQLite) | yes (same DB) |
| License | Apache-2.0 (+ heavy deps) | Apache-2.0 / MIT |

## Consequences

**Positive:**
- The agent's entire memory is **one portable `.db` file** — matches the product pitch exactly.
- No dual-write/consistency problem; atomic writes of fact + embedding + FTS.
- Lighter dependency tree; no Arrow/Lance churn.
- Honker (Phase 6) composes naturally for reactive maintenance + cron.

**Negative / Watch:**
- `sqlite-vec` does brute-force KNN (no ANN). Fine below ~100k vectors — a personal
  cross-project memory never reaches that. Revisit only if scale demands it.
- The `sqlite-vec` Rust binding is alpha (0.1.x) over a stable C library; pin the version,
  keep the wrapper surface minimal.
- `fastembed` adds 30–60 MB when the `semantic-search` feature is on — keep it feature-flagged.
