# CI/CD Pipeline

> **Status:** ✅ Complete
> **Category:** DevOps

---

## 1. Pipeline Overview

```
Push / PR
   │
   ├─ check.yml ─── cargo fmt, clippy, audit
   ├─ test.yml  ─── unit + integration tests (matrix: stable/beta)
   └─ release.yml ── build Docker image, push to GHCR, tag release
```

All workflows run on GitHub Actions.
Self-hosted runner optional for the [Docker build](60a_Docker_Build.md) (faster, avoids GHCR rate limits).

---

## 2. Check Workflow

```yaml
# .github/workflows/check.yml
name: Check

on:
  push:
    branches: [main, dev]
  pull_request:

env:
  CARGO_TERM_COLOR: always
  RUSTFLAGS: "-D warnings"

jobs:
  check:
    name: fmt + clippy + audit
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust stable
        uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - name: Cache cargo
        uses: Swatinem/rust-cache@v2

      - name: Format check
        run: cargo fmt --all -- --check

      - name: Clippy
        run: |
          cargo clippy --all-targets --all-features -- \
            -D clippy::correctness \
            -D clippy::suspicious \
            -W clippy::complexity \
            -W clippy::perf

      - name: Security audit
        uses: rustsec/audit-check@v1
        with:
          token: ${{ secrets.GITHUB_TOKEN }}
```

---

## 3. Test Workflow

```yaml
# .github/workflows/test.yml
name: Test

on:
  push:
    branches: [main, dev]
  pull_request:

env:
  CARGO_TERM_COLOR: always

jobs:
  test:
    name: Tests (${{ matrix.toolchain }})
    runs-on: ubuntu-latest

    strategy:
      matrix:
        toolchain: [stable, beta]

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust ${{ matrix.toolchain }}
        uses: dtolnay/rust-toolchain@master
        with:
          toolchain: ${{ matrix.toolchain }}

      - name: Cache cargo
        uses: Swatinem/rust-cache@v2
        with:
          key: ${{ matrix.toolchain }}

      - name: Install test dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y sqlite3 libsqlite3-dev

      - name: Run unit tests
        run: cargo test --workspace --lib -- --nocapture

      - name: Run integration tests
        run: cargo test --workspace --test '*' -- --nocapture
        env:
          TALON_TEST_MODE: "1"

      - name: Run doc tests
        run: cargo test --doc --workspace

  coverage:
    name: Coverage
    runs-on: ubuntu-latest
    if: github.ref == 'refs/heads/main'

    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: llvm-tools-preview

      - name: Install cargo-llvm-cov
        uses: taiki-e/install-action@cargo-llvm-cov

      - name: Cache cargo
        uses: Swatinem/rust-cache@v2

      - name: Generate coverage
        run: cargo llvm-cov --workspace --lcov --output-path lcov.info
        env:
          TALON_TEST_MODE: "1"

      - name: Upload to Codecov
        uses: codecov/codecov-action@v4
        with:
          files: lcov.info
          token: ${{ secrets.CODECOV_TOKEN }}
```

---

## 4. Release Workflow

```yaml
# .github/workflows/release.yml
name: Release

on:
  push:
    tags:
      - 'v*.*.*'

env:
  REGISTRY: ghcr.io
  IMAGE_NAME: ${{ github.repository }}

jobs:
  docker:
    name: Build & Push Docker
    runs-on: ubuntu-latest
    permissions:
      contents: read
      packages: write

    steps:
      - uses: actions/checkout@v4

      - name: Log in to GHCR
        uses: docker/login-action@v3
        with:
          registry: ${{ env.REGISTRY }}
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - name: Docker meta
        id: meta
        uses: docker/metadata-action@v5
        with:
          images: ${{ env.REGISTRY }}/${{ env.IMAGE_NAME }}
          tags: |
            type=semver,pattern={{version}}
            type=semver,pattern={{major}}.{{minor}}
            type=sha

      - name: Set up Docker Buildx
        uses: docker/setup-buildx-action@v3

      - name: Build and push
        uses: docker/build-push-action@v5
        with:
          context: .
          push: true
          tags: ${{ steps.meta.outputs.tags }}
          labels: ${{ steps.meta.outputs.labels }}
          cache-from: type=gha
          cache-to: type=gha,mode=max
          platforms: linux/amd64,linux/arm64

  binaries:
    name: Cross-compile binaries
    runs-on: ubuntu-latest
    strategy:
      matrix:
        include:
          - target: x86_64-unknown-linux-gnu
            os: linux
            arch: amd64
          - target: aarch64-unknown-linux-gnu
            os: linux
            arch: arm64
          - target: x86_64-apple-darwin
            os: macos
            arch: amd64

    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - name: Install cross
        run: cargo install cross --git https://github.com/cross-rs/cross

      - name: Build
        run: cross build --release --target ${{ matrix.target }}

      - name: Package
        run: |
          cd target/${{ matrix.target }}/release
          tar czf talon-${{ github.ref_name }}-${{ matrix.os }}-${{ matrix.arch }}.tar.gz talon
          sha256sum *.tar.gz > checksums.txt

      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: binary-${{ matrix.target }}
          path: target/${{ matrix.target }}/release/*.tar.gz

  github-release:
    name: Create GitHub Release
    needs: [docker, binaries]
    runs-on: ubuntu-latest
    permissions:
      contents: write

    steps:
      - uses: actions/download-artifact@v4
        with:
          pattern: binary-*
          merge-multiple: true
          path: artifacts/

      - name: Create release
        uses: softprops/action-gh-release@v2
        with:
          files: artifacts/*
          generate_release_notes: true
```

---

## 5. Branch Strategy

```
main        → production (protected, requires PR + passing CI)
dev         → integration branch (auto-deploys to staging)
feat/*      → feature branches (PR into dev)
fix/*       → bug fix branches (PR into dev or main for hotfixes)
```

Branch protection rules for `main`:
- Require PR review (1 approver)
- Require status checks: `check / check`, `test / Tests (stable)`
- No force-push
- No direct commits

---

## 6. Local Pre-commit Hooks

```bash
# .git/hooks/pre-commit (or use cargo-husky)
#!/bin/bash
set -e

echo "Running pre-commit checks..."
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --lib --quiet
echo "All checks passed."
```

Install via:
```bash
cp scripts/pre-commit .git/hooks/pre-commit
chmod +x .git/hooks/pre-commit
```

Or use `cargo-husky`:
```toml
# Cargo.toml
[dev-dependencies]
cargo-husky = { version = "1", default-features = false, features = ["user-hooks"] }
```

---

## 7. Dependency Update Bot

```yaml
# .github/dependabot.yml
version: 2
updates:
  - package-ecosystem: cargo
    directory: "/"
    schedule:
      interval: weekly
    ignore:
      - dependency-name: "*"
        update-types: ["version-update:semver-major"]  # no auto-major bumps

  - package-ecosystem: docker
    directory: "/"
    schedule:
      interval: weekly

  - package-ecosystem: github-actions
    directory: "/"
    schedule:
      interval: weekly
```
---

## Related Documents

### See Also
- [CI/CD Pipeline (main)](62_CI_CD_Pipeline.md)
- [Build System / Cargo Workspace](60_Build_System_Cargo_Workspace.md)

