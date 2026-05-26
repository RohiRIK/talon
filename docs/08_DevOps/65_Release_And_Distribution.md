# Release & Distribution

> **Status:** ✅ Complete
> **Category:** DevOps

---

## 1. Release Artifacts

Talon ships in four forms:

| Artifact | Target audience | Size |
|----------|----------------|------|
| [Docker image (GHCR) | Server/homelab deployment](61_Docker_And_Container_Deployment.md) | ~85MB |
| Linux binary (x86_64) | Direct install, CI runners | ~15MB |
| Linux binary (arm64) | Raspberry Pi, ARM servers | ~15MB |
| macOS binary (x86_64) | Developer workstations | ~16MB |

All binaries are statically linked against musl (Linux) or compiled
for the target platform (macOS).

---

## 2. Version Scheme

Talon uses **Semantic Versioning**: `MAJOR.MINOR.PATCH`

```
0.x.y   = Pre-stable (breaking changes allowed)
1.0.0   = First stable release (API contract locked)
1.x.y   = Stable (no breaking changes within major)
```

Git tags: `v0.1.0`, `v1.0.0`, etc.
Docker tags mirror Git tags: `ghcr.io/yourorg/talon:v1.0.0`
Plus rolling tags: `:latest`, `:1`, `:1.0`

---

## 3. Release Checklist

Before tagging a release:

```markdown
## Pre-Release
- [ ] All tests pass: `cargo test --workspace`
- [ ] Clippy clean: `cargo clippy --all-targets -- -D warnings`
- [ ] No security advisories: `cargo audit`
- [ ] CHANGELOG.md updated with new version section
- [ ] Version bumped in `Cargo.toml` (workspace root)
- [ ] Migration scripts added if DB schema changed

## Tagging
- [ ] `git tag -s v1.2.3 -m "Release v1.2.3"`
- [ ] `git push origin v1.2.3`
- [ ] CI/CD release workflow triggered automatically

## Post-Release
- [ ] GitHub Release created with notes (auto from workflow)
- [ ] Docker image pushed to GHCR
- [ ] Binaries attached to GitHub Release
- [ ] Announce in relevant channels
```

---

## 4. CHANGELOG Format

```markdown
# Changelog

All notable changes to Talon are documented here.
Format based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]
### Added
- TUI voice mode toggle

## [1.2.0] - 2025-02-01
### Added
- Discord gateway
- WASM plugin support

### Changed
- FTS5 search now returns bookend context by default
- `delegate_task` max concurrency now configurable

### Fixed
- Cron jobs not persisting after DB vacuum
- Telegram photo delivery failing for .webp files

### Removed
- Legacy `execute_code` Python sandbox (replaced by Rust native)

## [1.1.0] - 2025-01-15
...
```

---

## 5. Binary Distribution via install script

```bash
#!/bin/bash
# install.sh — one-line installer
# Usage: curl -sSf https://talon.sh/install | bash

set -euo pipefail

REPO="yourorg/talon"
INSTALL_DIR="${TALON_INSTALL_DIR:-$HOME/.local/bin}"

# Detect platform
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$ARCH" in
    x86_64)  ARCH="amd64" ;;
    aarch64) ARCH="arm64" ;;
    arm64)   ARCH="arm64" ;;  # macOS M-series
    *)       echo "Unsupported arch: $ARCH"; exit 1 ;;
esac

# Get latest release
LATEST=$(curl -sSf "https://api.github.com/repos/$REPO/releases/latest" \
    | grep '"tag_name"' | head -1 | cut -d'"' -f4)

FILENAME="talon-${LATEST}-${OS}-${ARCH}.tar.gz"
URL="https://github.com/$REPO/releases/download/$LATEST/$FILENAME"

echo "Installing Talon $LATEST for $OS/$ARCH..."
curl -sSfL "$URL" | tar xz -C /tmp
install -m 755 /tmp/talon "$INSTALL_DIR/talon"
rm -f /tmp/talon

echo "Talon installed to $INSTALL_DIR/talon"
echo "Run: talon --help"
```

---

## 6. Homebrew Formula (macOS)

```ruby
# Formula/talon.rb
class Talon < Formula
  desc "Autonomous AI agent — native Rust"
  homepage "https://github.com/yourorg/talon"
  version "1.0.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/yourorg/talon/releases/download/v1.0.0/talon-v1.0.0-macos-arm64.tar.gz"
      sha256 "REPLACE_WITH_ACTUAL_SHA256"
    else
      url "https://github.com/yourorg/talon/releases/download/v1.0.0/talon-v1.0.0-macos-amd64.tar.gz"
      sha256 "REPLACE_WITH_ACTUAL_SHA256"
    end
  end

  def install
    bin.install "talon"
  end

  test do
    assert_match "Talon", shell_output("#{bin}/talon --version")
  end
end
```

---

## 7. Docker Distribution

```bash
# Pull latest
docker pull ghcr.io/yourorg/talon:latest

# Pull specific version
docker pull ghcr.io/yourorg/talon:v1.0.0

# Run interactively
docker run -it --rm \
  -e ANTHROPIC_API_KEY="$ANTHROPIC_API_KEY" \
  -v ~/.talon:/data \
  ghcr.io/yourorg/talon:latest chat

# Run as service (Telegram bot)
docker run -d --name talon \
  --restart unless-stopped \
  -e ANTHROPIC_API_KEY="$ANTHROPIC_API_KEY" \
  -e TALON_TELEGRAM_BOT_TOKEN="$TELEGRAM_BOT_TOKEN" \
  -v talon_data:/data \
  ghcr.io/yourorg/talon:latest serve
```

---

## 8. Version Command

```rust
// talon/src/main.rs
#[derive(Parser)]
enum Command {
    /// Print version info
    Version,
    // ...
}

fn handle_version() {
    println!("Talon {}", env!("CARGO_PKG_VERSION"));
    println!("Built: {}", env!("TALON_BUILD_DATE"));      // set in build.rs
    println!("Commit: {}", env!("TALON_GIT_HASH"));        // set in build.rs
    println!("Features: {}", env!("TALON_FEATURES"));      // set in build.rs
    println!("Rust: {}", env!("TALON_RUSTC_VERSION"));     // set in build.rs
}
```

```rust
// build.rs
fn main() {
    // Git hash
    let hash = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    println!("cargo:rustc-env=TALON_GIT_HASH={hash}");
    println!("cargo:rustc-env=TALON_BUILD_DATE={}", chrono::Utc::now().format("%Y-%m-%d"));
    println!("cargo:rustc-env=TALON_RUSTC_VERSION={}", rustc_version::version().unwrap());
    println!("cargo:rerun-if-changed=.git/HEAD");
}
```
---

## Related Documents

### Depends On
- [CI/CD Pipeline](62_CI_CD_Pipeline.md)
- [Build System / Cargo Workspace](60_Build_System_Cargo_Workspace.md)

### See Also
- [Docker & Container Deployment](61_Docker_And_Container_Deployment.md)

