# ADR 0002 — thiserror in Libraries, anyhow in the Binary

**Status:** Accepted  
**Date:** 2026-05-27

## Context

Rust error handling has two dominant patterns:
- `thiserror`: generates `std::error::Error` impls from typed enum variants — best for library crates that expose error types to callers
- `anyhow`: wraps any error in a dynamic `anyhow::Error` with a chain of context — best for application code where the caller just wants to surface the error to a user

## Decision

- **Library crates** (`talon-core`, `talon-llm`, `talon-memory`, `talon-tools`, `talon-gateway`, `talon-plugins`): use `thiserror` only. Each crate defines a `pub enum XxxError` with `#[derive(thiserror::Error)]`.
- **Binary crate** (`talon`): use `anyhow` only. `main()` returns `anyhow::Result<()>`. Context is attached with `.context("...")`.
- **Never mix** `anyhow` and `thiserror` in the same crate. `clippy` enforces this indirectly; code review is the gate.

## Consequences

**Positive:**
- Library users get typed errors they can match on (`CoreError::Timeout`, `LlmError::RateLimited`)
- Binary gets rich error chains without boilerplate
- Clear boundary: library = typed, binary = contextual

**Negative / Watch:**
- Converting `thiserror` errors to `anyhow` at the binary boundary requires `?` (implicit `From` conversion) — this works seamlessly as long as error types implement `std::error::Error`, which `thiserror` guarantees
