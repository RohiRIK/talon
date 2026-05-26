# CI/CD Pipeline

> **Status:** ✅ Complete
> **Category:** DevOps

---

## 1. GitHub Actions Workflow

```yaml
# .github/workflows/ci.yml
name: CI

on:
  push:
    branches: [main, develop]
  pull_request:
    branches: [main]

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  test:
    name: Test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2

      - name: Run tests
        run: cargo test --workspace --profile ci

      - name: Clippy
        run: cargo clippy --workspace -- -D warnings

      - name: Format check
        run: cargo fmt --all -- --check

  security:
    name: Security Audit
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo install cargo-audit
      - run: cargo audit

  build-release:
    name: Build Release
    needs: [test, security]
    runs-on: ubuntu-latest
    if: github.ref == 'refs/heads/main'
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2

      - name: Build release binary
        run: cargo build --release

      - name: Build Docker image
        run: docker build -t talon:${{ github.sha }} .

      - name: Push to registry
        run: |
          echo "${{ secrets.REGISTRY_PASSWORD }}" | docker login ghcr.io -u ${{ github.actor }} --password-stdin
          docker tag talon:${{ github.sha }} ghcr.io/${{ github.repository_owner }}/talon:latest
          docker push ghcr.io/${{ github.repository_owner }}/talon:latest
```

---

## 2. Test Organization

```
tests/
├── unit/                   # Unit tests (alongside source via #[cfg(test)])
├── integration/            # Multi-crate integration tests
│   ├── test_agent_loop.rs
│   ├── test_tool_execution.rs
│   └── test_memory.rs
└── e2e/                    # End-to-end tests (excluded from default test run)
    └── test_full_session.rs
```

```toml
# .cargo/config.toml
[alias]
test-unit = "test --workspace --lib"
test-integration = "test --workspace --tests"
test-e2e = "test --test '*' -- --test-threads=1"
```

---

## 3. Test Helpers

```rust
// tests/integration/helpers.rs

pub async fn test_agent() -> AgentLoop {
    AgentLoop::new(AgentConfig {
        llm: LlmConfig::mock(),         // Mock LLM — no API calls
        memory: MemoryConfig::temp(),   // tmpdir SQLite
        tools: ToolsConfig::safe_only(),// No shell tools in tests
        ..Default::default()
    }).await.unwrap()
}

pub struct MockLlm {
    responses: VecDeque<String>,
}

#[async_trait]
impl LlmProvider for MockLlm {
    async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse {
            content: self.responses.pop_front().unwrap_or_default(),
            tool_calls: vec![],
            stop_reason: StopReason::EndTurn,
            usage: None,
        })
    }
}
```

---

## 4. Coverage

```bash
# Install tarpaulin
cargo install cargo-tarpaulin

# Run with coverage
cargo tarpaulin --workspace --out Html --output-dir coverage/

# CI: fail if coverage drops below 70%
cargo tarpaulin --workspace --fail-under 70
```

---

## 5. Release Checklist

```markdown
## Release Checklist
- [ ] Version bumped in workspace Cargo.toml
- [ ] CHANGELOG.md updated
- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo audit` clean
- [ ] Docker build passes
- [ ] Manual smoke test (Telegram + CLI)
- [ ] Git tag: `git tag v1.x.x && git push --tags`
- [ ] GitHub release created with binary artifacts
```
---

## Related Documents

### Depends On
- [Build System / Cargo Workspace](60_Build_System_Cargo_Workspace.md)
- [Test Strategy](../03_Migration_Strategy/26_Test_Strategy.md)

### See Also
- [Docker & Container Deployment](61_Docker_And_Container_Deployment.md)
- [Release & Distribution](65_Release_And_Distribution.md)

