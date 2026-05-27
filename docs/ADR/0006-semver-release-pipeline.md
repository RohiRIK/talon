# ADR 0006 — Semver + Release Pipeline Design

**Status:** Accepted  
**Date:** 2026-05-27

## Context

Talon needs a release pipeline that is: secure by default, reproducible, auditable, and low-maintenance. The binary is distributed via multiple channels (GitHub Releases, crates.io, Homebrew, AUR, Docker Hub).

## Decision

### Version Scheme

- **Semver** (`MAJOR.MINOR.PATCH`) for all crates and the binary
- Single source of truth: `[workspace.package] version` in root `Cargo.toml`
- Start at `0.1.0` on Phase 0 completion; `1.0.0` only when all Final Acceptance Criteria pass

### Tagging + Release Triggers

- Tags: `v0.1.0`, `v0.2.0` — annotated (`git tag -a`)
- Release workflow fires only on `v*` tag push — never on branch push
- Manual and deliberate: a human pushes the tag, not CI

### Supply-Chain Security (non-negotiable)

| Control | Implementation |
|---------|---------------|
| Deny-all permissions | `permissions: {}` at workflow top-level |
| SHA-pinned actions | Every action pinned to exact commit SHA; Dependabot bumps weekly |
| Keyless binary signing | `cosign` via GitHub OIDC — no stored private key |
| SLSA L2 provenance | `actions/attest-build-provenance` for every artifact |
| Checksums | `SHA256SUMS` published with every release |
| CVE gating | `cargo audit` + `cargo deny check` on every PR |
| crates.io publishing | Trusted publishing (OIDC) — no stored API token |
| Docker signing | `cosign sign` on image digest after push |

### Changelog

- `git-cliff` generates `CHANGELOG.md` from conventional commits
- Config: `cliff.toml` at workspace root
- CHANGELOG.md is committed; never hand-edited after Phase 0

### Distribution Channels

| Channel | Tool | What ships |
|---------|------|-----------|
| GitHub Releases | `cargo dist` | Pre-built binaries + signatures + SHA256SUMS |
| crates.io | `cargo publish` | Library crates only (core/llm/memory/tools) |
| Homebrew | `cargo dist` | macOS/Linux tap formula |
| AUR | Manual PKGBUILD | Arch Linux |
| Docker Hub | `docker buildx` + cosign | OCI image, multi-arch |

**NOT npm.** Talon is a Rust binary — npm is not a distribution target.

## Consequences

- More workflow YAML than a naive pipeline — justified by the security posture
- Dry-run test (`v0.0.1-test` tag) required before the first real release to validate signing + attestation
- crates.io trusted publishing requires one-time dashboard configuration (done during Phase 0)
