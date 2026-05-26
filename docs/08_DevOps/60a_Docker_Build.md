# Docker Build

> **Status:** ✅ Complete
> **Category:** DevOps

---

## 1. Multi-Stage Build Strategy

Talon's Docker build uses three stages:

```
Stage 1: chef (cargo-chef)   → dependency cache layer
Stage 2: builder             → compile release binary
Stage 3: runtime             → minimal runtime image (~10MB)
```

`cargo-chef` caches dependencies separately from source code.
Rebuilding after code changes skips the ~5 minute dep compile.

---

## 2. Dockerfile

```dockerfile
# syntax=docker/dockerfile:1.7

# ─── Stage 1: Dependency planner ─────────────────────────────────────────────
FROM rust:1.79-slim-bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /build

# ─── Stage 2: Dependency cache ───────────────────────────────────────────────
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /build/recipe.json recipe.json
# Cache deps (only invalidated when Cargo.toml/Cargo.lock changes)
RUN cargo chef cook --release --recipe-path recipe.json

# Build app
COPY . .

# Optional: enable embeddings feature
ARG FEATURES=""
RUN cargo build --release --workspace \
    $([ -n "$FEATURES" ] && echo "--features $FEATURES") \
    2>&1 | tail -5

# ─── Stage 3: Runtime image ──────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

# Runtime dependencies only
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Non-root user
RUN groupadd -r talon && useradd -r -g talon -s /bin/false talon

WORKDIR /app

# Copy binary
COPY --from=builder /build/target/release/talon /usr/local/bin/talon

# Copy default config template (user mounts their actual config)
COPY docker/config.template.toml /app/config.template.toml
COPY docker/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

# Data dir for SQLite + logs
RUN mkdir -p /data && chown talon:talon /data

USER talon

EXPOSE 8080

VOLUME ["/data", "/app/config"]

ENTRYPOINT ["/entrypoint.sh"]
CMD ["serve"]
```

---

## 3. Entrypoint Script

```bash
#!/bin/bash
# docker/entrypoint.sh
set -euo pipefail

CONFIG_PATH="${TALON_CONFIG:-/app/config/config.toml}"

# Generate config from template + env vars if no config mounted
if [ ! -f "$CONFIG_PATH" ]; then
    echo "No config found at $CONFIG_PATH, generating from environment..."
    mkdir -p "$(dirname "$CONFIG_PATH")"
    envsubst < /app/config.template.toml > "$CONFIG_PATH"
fi

# Run migrations on startup
talon migrate --config "$CONFIG_PATH"

# Hand off to CMD
exec talon "$@" --config "$CONFIG_PATH"
```

---

## 4. docker-compose.yml

```yaml
# docker-compose.yml
services:
  talon:
    image: ghcr.io/yourorg/talon:latest
    build:
      context: .
      dockerfile: Dockerfile
      args:
        FEATURES: ""          # set to "embeddings" to enable semantic search
    environment:
      TALON_LOG_LEVEL: info
      TALON_DATA_DIR: /data

      # LLM providers (at least one required)
      ANTHROPIC_API_KEY: ${ANTHROPIC_API_KEY:-}
      OPENAI_API_KEY: ${OPENAI_API_KEY:-}
      OPENROUTER_API_KEY: ${OPENROUTER_API_KEY:-}

      # Gateway
      TALON_TELEGRAM_BOT_TOKEN: ${TELEGRAM_BOT_TOKEN:-}
      TALON_HTTP_PORT: "8080"

    volumes:
      - talon_data:/data
      - ./config:/app/config:ro

    ports:
      - "127.0.0.1:8080:8080"   # HTTP gateway (localhost only)

    restart: unless-stopped

    security_opt:
      - no-new-privileges:true
      - seccomp:./docker/seccomp.json   # custom seccomp profile

    cap_drop:
      - ALL

    read_only: true
    tmpfs:
      - /tmp:size=100m

    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 30s
      timeout: 5s
      retries: 3
      start_period: 10s

    logging:
      driver: json-file
      options:
        max-size: "50m"
        max-file: "3"

  # Optional: Qdrant for semantic search
  qdrant:
    image: qdrant/qdrant:v1.9.7
    profiles: ["embeddings"]
    volumes:
      - qdrant_data:/qdrant/storage
    ports:
      - "127.0.0.1:6333:6333"
    restart: unless-stopped

volumes:
  talon_data:
  qdrant_data:
```

---

## 5. Seccomp Profile

`docker/seccomp.json` — minimal syscall allowlist:

```json
{
  "defaultAction": "SCMP_ACT_ERRNO",
  "syscalls": [
    {
      "names": [
        "read", "write", "open", "close", "stat", "fstat",
        "lstat", "poll", "lseek", "mmap", "mprotect", "munmap",
        "brk", "rt_sigaction", "rt_sigprocmask", "ioctl",
        "access", "socket", "connect", "accept", "sendto",
        "recvfrom", "sendmsg", "recvmsg", "bind", "listen",
        "getsockname", "getpeername", "setsockopt", "getsockopt",
        "clone", "fork", "vfork", "execve", "exit", "wait4",
        "kill", "uname", "fcntl", "flock", "fsync", "fdatasync",
        "truncate", "ftruncate", "getdents", "getcwd", "chdir",
        "rename", "mkdir", "rmdir", "creat", "link", "unlink",
        "symlink", "readlink", "chmod", "chown", "umask",
        "futex", "nanosleep", "getpid", "getppid",
        "getuid", "geteuid", "getgid", "getegid",
        "clock_gettime", "clock_getres",
        "epoll_create", "epoll_ctl", "epoll_wait", "epoll_pwait",
        "eventfd", "eventfd2", "timerfd_create", "timerfd_settime",
        "timerfd_gettime", "signalfd", "signalfd4",
        "pipe", "pipe2", "splice", "tee",
        "getrandom", "pread64", "pwrite64"
      ],
      "action": "SCMP_ACT_ALLOW"
    }
  ]
}
```

---

## 6. Build Targets

```makefile
# Makefile
IMAGE := ghcr.io/yourorg/talon
VERSION := $(shell git describe --tags --always --dirty)

.PHONY: build build-embeddings push run

build:
	docker build -t $(IMAGE):$(VERSION) -t $(IMAGE):latest .

build-embeddings:
	docker build --build-arg FEATURES=embeddings \
	  -t $(IMAGE):$(VERSION)-embeddings .

push:
	docker push $(IMAGE):$(VERSION)
	docker push $(IMAGE):latest

run:
	docker compose up -d

logs:
	docker compose logs -f talon
```

---

## 7. Image Size Comparison

| Stage | Approach | Approx. size |
|-------|----------|-------------|
| builder | rust:slim-bookworm | ~1.5GB |
| runtime (glibc) | debian:bookworm-slim | ~85MB |
| runtime (musl) | scratch + binary | ~12MB |

For smallest image, build with musl target:

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
# Copy binary into FROM scratch
```

Tradeoff: musl has slightly slower allocator. Unless image size is critical,
`debian:bookworm-slim` is simpler and avoids musl edge cases with `ort` (ONNX).
---

## Related Documents

### Depends On
- [Docker & Container Deployment](61_Docker_And_Container_Deployment.md)

### See Also
- [Build System / Cargo Workspace](60_Build_System_Cargo_Workspace.md)
- [Security Model](../02_Architecture/20_Security_Model.md)

