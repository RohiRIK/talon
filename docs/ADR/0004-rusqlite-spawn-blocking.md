# ADR 0004 — rusqlite via spawn_blocking / deadpool-sqlite

**Status:** Accepted  
**Date:** 2026-05-27

## Context

`rusqlite::Connection` is `!Send` — the underlying SQLite C library is not thread-safe when the same connection is used concurrently. Wrapping `Connection` in `tokio::sync::Mutex` does not work: `Mutex<T>` requires `T: Send`, which `Connection` is not. The compiler will reject it.

Additionally, SQLite operations are blocking I/O. Running them directly in an async task stalls the Tokio thread pool, which can starve other tasks.

## Decision

All SQLite access uses one of two patterns:

**Pattern A — deadpool-sqlite (preferred for multi-connection workloads):**
```rust
// Pool is Send + Sync; connections are managed on blocking threads
let conn = pool.get().await?;
let result = conn.interact(|conn| {
    // This closure runs on a blocking thread.
    // conn is &mut Connection, which IS Send in this scope.
    // MUST NOT .await inside this closure.
    conn.execute("INSERT ...", params![...])
}).await??;
```

**Pattern B — spawn_blocking (for one-off operations):**
```rust
tokio::task::spawn_blocking(move || {
    let conn = Connection::open(&db_path)?;
    // blocking DB work here
    // Return a value; do not hold conn past this closure
    Ok::<_, rusqlite::Error>(result)
}).await?
```

## Hard Rules

1. Never hold a `Connection` across an `.await` point
2. Never put `Connection` in `tokio::sync::Mutex`
3. Never call network I/O inside a `spawn_blocking` or `interact` closure
4. All `interact` closures must be `Send + 'static` (no borrows from outside)

## Consequences

- Slightly more verbose DB code (the `interact` / `spawn_blocking` wrapper)
- No data races on the DB connection
- Tokio thread pool is not starved by blocking SQLite calls
