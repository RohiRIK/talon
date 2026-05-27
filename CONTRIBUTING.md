# Contributing to Talon

Thank you for considering a contribution. Talon is in early development (Phase 0); the contribution bar is intentionally high right now — architectural decisions are locked and the codebase is being built to a specific spec.

---

## Before You Start

1. **Read the spec.** The architecture is documented in detail. Start with:
   - [`PLAN.md`](PLAN.md) — phased build plan with exit gates
   - [`CLAUDE.md`](CLAUDE.md) — invariants, anti-patterns, load-bearing types
   - [`docs/ADR/`](docs/ADR/) — architectural decisions and their rationale
   - [`roadmap.md`](roadmap.md) — dependency graph and timeline

2. **Check the current phase.** See the Phase Status table in `CLAUDE.md`. Only contribute to the current or immediately next phase — out-of-order contributions will not be accepted.

3. **Open an issue first** for non-trivial changes. Describe what you want to change and why. Large PRs without a prior discussion will be closed.

---

## Development Setup

```bash
# Toolchain
rustup update stable
rustup component add rustfmt clippy

# Dev tools
cargo install cargo-nextest cargo-audit cargo-deny git-cliff --locked

# Optional: git hooks
cargo install lefthook --locked
lefthook install

# Verify the workspace builds
cargo build --workspace
cargo nextest run --workspace
```

---

## Invariants (Hard Rules)

These are not preferences — they are invariants. Any PR that violates them will be rejected:

| Rule | Reason |
|------|--------|
| No `.unwrap()` outside `#[cfg(test)]` | Crashes in production |
| `rusqlite::Connection` never across `.await` | `Connection` is `!Send` |
| `async-trait` crate is banned | Edition 2024 has native async fn in traits |
| `Arc<dyn Tool>` not `Arc<Box<dyn Tool>>` | Double indirection is wrong |
| `thiserror` in libraries, `anyhow` in binary | See ADR 0002 |
| `cargo nextest run`, never `cargo test` | Consistency with CI |
| 7 load-bearing types defined once in their home crate | See `CLAUDE.md` |

---

## Pull Request Checklist

Before opening a PR:

- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used` clean
- [ ] `cargo nextest run --workspace` green
- [ ] `cargo audit` clean
- [ ] `cargo deny check` clean
- [ ] New code has tests (80% coverage target)
- [ ] No `.unwrap()` outside test modules
- [ ] Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/)

---

## Commit Format

```
<type>(<scope>): <description>

[optional body]
```

Types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `perf`, `ci`

Scopes map to crate names: `core`, `llm`, `memory`, `tools`, `gateway`, `plugins`, `ci`, `release`

Examples:
```
feat(core): add ApprovalMembrane per-invocation check
fix(memory): prevent Connection from crossing await in migrate()
docs(adr): add ADR 0007 for tool dispatch strategy
```

---

## Security

Do not open public issues for security vulnerabilities. See [`.github/SECURITY.md`](.github/SECURITY.md).

---

## License

By contributing, you agree that your contributions will be licensed under the same dual MIT/Apache-2.0 license as the project.
