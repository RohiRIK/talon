# Docker & Container Deployment

> **Status:** ✅ Complete
> **Category:** DevOps

---

## 1. Multi-Stage Dockerfile

```dockerfile
# Stage 1: Build
FROM rust:1.81-slim-bookworm AS builder

WORKDIR /build

# Install system deps
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Cache dependencies
COPY Cargo.toml Cargo.lock ./
COPY talon-core/Cargo.toml talon-core/
COPY talon-llm/Cargo.toml talon-llm/
COPY talon-memory/Cargo.toml talon-memory/
COPY talon-gateway/Cargo.toml talon-gateway/
COPY talon-plugins/Cargo.toml talon-plugins/
COPY talon-tools/Cargo.toml talon-tools/

# Dummy build for dependency caching
RUN mkdir -p src talon-core/src talon-llm/src talon-memory/src \
             talon-gateway/src talon-plugins/src talon-tools/src
RUN echo "fn main() {}" > src/main.rs
RUN for d in talon-core talon-llm talon-memory talon-gateway talon-plugins talon-tools; \
    do echo "fn main() {}" > $d/src/lib.rs; done
RUN cargo build --release 2>/dev/null; true

# Real build
COPY . .
RUN touch src/main.rs  # Force rebuild
RUN cargo build --release

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    # For edge-tts voice support (optional):
    python3-minimal \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 -s /bin/bash talon

COPY --from=builder /build/target/release/talon /usr/local/bin/talon

USER talon
WORKDIR /home/talon

# Data directory (mount external volume here)
RUN mkdir -p /home/talon/.talon/profiles/default/{db,memories,skills}

VOLUME ["/home/talon/.talon"]

ENTRYPOINT ["talon"]
CMD ["--profile", "default", "--gateway", "http", "--port", "8080"]
```

---

## 2. docker-compose.yml

```yaml
version: "3.9"

services:
  talon:
    build: .
    image: talon:latest
    container_name: talon
    restart: unless-stopped

    environment:
      ANTHROPIC_API_KEY: ${ANTHROPIC_API_KEY}
      TELEGRAM_BOT_TOKEN: ${TELEGRAM_BOT_TOKEN}
      OPENAI_API_KEY: ${OPENAI_API_KEY}
      RUST_LOG: "talon=info,talon_core=debug"

    volumes:
      - talon_data:/home/talon/.talon
      - ./config:/home/talon/.talon/profiles/default/config.toml:ro

    ports:
      - "8080:8080"   # HTTP API

    healthcheck:
      test: ["CMD", "talon", "health"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 10s

    # Security hardening
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    read_only: true
    tmpfs:
      - /tmp:noexec,nosuid,size=256m

volumes:
  talon_data:
    driver: local
```

---

## 3. Code Execution Sandbox

For the `terminal` tool, Talon spawns subprocess commands.
In Docker, these run with restricted capabilities via seccomp:

```json
// seccomp-profile.json (subset of default Docker profile)
{
  "defaultAction": "SCMP_ACT_ERRNO",
  "syscalls": [
    { "names": ["read", "write", "open", "close", "stat", "fstat",
                "mmap", "mprotect", "exit", "exit_group", "futex",
                "clone", "fork", "execve", "wait4", "kill",
                "getcwd", "chdir", "pipe", "dup", "dup2",
                "getpid", "getppid", "getuid", "getgid"],
      "action": "SCMP_ACT_ALLOW" }
  ]
}
```

For Talon's code_exec tool (running untrusted LLM-generated code):

```yaml
code_exec_sandbox:
  image: python:3.12-slim
  network_mode: none       # No network access
  read_only: true
  tmpfs: ["/tmp"]
  mem_limit: "256m"
  cpu_quota: 50000         # 0.5 CPU
  security_opt:
    - no-new-privileges:true
  cap_drop: [ALL]
```

---

## 4. Health Check

```rust
// talon health → exits 0 if healthy
pub async fn health_check(config: &Config) -> anyhow::Result<()> {
    // Check DB
    let db = Database::open(&config.db_path)?;
    db.ping().await?;

    // Check LLM provider reachability (quick probe)
    let provider = build_provider(config).await?;
    provider.ping().await?;

    println!("Talon: healthy");
    Ok(())
}
```
---

## Related Documents

### Depends On
- [Security Model](../02_Architecture/20_Security_Model.md)

### See Also
- [Docker Build](60a_Docker_Build.md)
- [CI/CD Pipeline](62_CI_CD_Pipeline.md)
- [Terminal Tool (Sandbox)](../04_Core_Features/30a_Terminal_Tool.md)

