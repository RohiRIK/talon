use rusqlite::Connection;

use crate::error::MemoryError;

/// All migrations in order. Each entry is `(version, sql)`.
/// Only migrations with version > the current max in schema_migrations are applied.
static MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("schema.sql")),
    (2, include_str!("schema_ltm.sql")),
    (3, include_str!("schema_ltm_tags.sql")),
    (4, include_str!("schema_cron.sql")),
    (5, include_str!("schema_runs.sql")),
    (6, include_str!("schema_secrets.sql")),
    (7, include_str!("schema_tokens.sql")),
    (8, include_str!("schema_hooks.sql")),
];

/// Run all pending migrations on an open connection.
/// Safe to call on every startup: already-applied versions are skipped.
pub fn run(conn: &Connection) -> Result<(), MemoryError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version    INTEGER PRIMARY KEY,
             applied_at INTEGER NOT NULL DEFAULT (unixepoch())
         );",
    )?;

    let current_version: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    for &(version, sql) in MIGRATIONS {
        if version <= current_version {
            continue;
        }
        conn.execute_batch(sql)
            .map_err(|e| MemoryError::MigrationFailed(format!("v{version}: {e}")))?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations (version) VALUES (?1)",
            [version],
        )?;
        tracing::info!("applied migration v{version}");
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn in_memory() -> Connection {
        // vec_memories (migration v2) is a vec0 virtual table — the extension must
        // be registered before the connection opens (auto-extension applies at open).
        crate::register_sqlite_vec();
        Connection::open(":memory:").expect("in-memory db")
    }

    #[test]
    fn run_migrations_creates_all_tables() {
        let conn = in_memory();
        run(&conn).expect("migrations");

        let tables: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .expect("prepare");
            stmt.query_map([], |r| r.get(0))
                .expect("query")
                .collect::<Result<Vec<_>, _>>()
                .expect("collect")
        };

        for expected in &[
            "messages",
            "sessions",
            "schema_migrations",
            "tool_calls",
            "skills",
            "user_facts",
            "memories",
            "memories_fts",
            "vec_memories",
            "cron_jobs",
            "cron_runs",
            "secrets",
            "api_tokens",
            "webhooks",
        ] {
            assert!(
                tables.iter().any(|t| t == expected),
                "missing table: {expected}"
            );
        }
    }

    #[test]
    fn run_migrations_is_idempotent() {
        let conn = in_memory();
        run(&conn).expect("first run");
        run(&conn).expect("second run — idempotent");
    }

    #[test]
    fn run_records_version_in_schema_migrations() {
        let conn = in_memory();
        run(&conn).expect("migrations");

        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
                r.get(0)
            })
            .expect("query");
        assert_eq!(version, 8);
    }

    #[test]
    fn v5_applies_on_top_of_existing_v4_database() {
        // Simulate an existing install: apply v1–v4 only, then run() — only v5
        // (cron_runs) should be applied, leaving prior data intact.
        let conn = in_memory();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                 version    INTEGER PRIMARY KEY,
                 applied_at INTEGER NOT NULL DEFAULT (unixepoch())
             );",
        )
        .expect("bootstrap");
        for &(version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v <= 4) {
            conn.execute_batch(sql).expect("apply legacy");
            conn.execute(
                "INSERT OR IGNORE INTO schema_migrations (version) VALUES (?1)",
                [version],
            )
            .expect("record legacy");
        }
        conn.execute(
            "INSERT INTO cron_jobs (id, schedule, prompt, session_id) VALUES ('j1', '\"daily\"', 'p', 's')",
            [],
        )
        .expect("seed job");

        run(&conn).expect("upgrade past v4");

        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
                r.get(0)
            })
            .expect("version");
        assert_eq!(version, 8);

        // cron_runs exists and is empty; existing job survived.
        let runs: i64 = conn
            .query_row("SELECT COUNT(*) FROM cron_runs", [], |r| r.get(0))
            .expect("cron_runs query");
        assert_eq!(runs, 0);
        let jobs: i64 = conn
            .query_row("SELECT COUNT(*) FROM cron_jobs", [], |r| r.get(0))
            .expect("cron_jobs query");
        assert_eq!(jobs, 1);
    }

    #[test]
    fn v6_applies_on_top_of_existing_v5_database() {
        // Existing install at v5 (web console): run() should apply only v6,
        // creating the empty secrets table and leaving run history intact.
        let conn = in_memory();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                 version    INTEGER PRIMARY KEY,
                 applied_at INTEGER NOT NULL DEFAULT (unixepoch())
             );",
        )
        .expect("bootstrap");
        for &(version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v <= 5) {
            conn.execute_batch(sql).expect("apply legacy");
            conn.execute(
                "INSERT OR IGNORE INTO schema_migrations (version) VALUES (?1)",
                [version],
            )
            .expect("record legacy");
        }
        conn.execute(
            "INSERT INTO cron_jobs (id, schedule, prompt, session_id) VALUES ('j1', '\"daily\"', 'p', 's')",
            [],
        )
        .expect("seed job");
        conn.execute(
            "INSERT INTO cron_runs (id, job_id, started_at, status) VALUES ('r1', 'j1', '2026-06-01T00:00:00Z', 'success')",
            [],
        )
        .expect("seed run");

        run(&conn).expect("upgrade to v6");

        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
                r.get(0)
            })
            .expect("version");
        assert_eq!(version, 8);

        let secrets: i64 = conn
            .query_row("SELECT COUNT(*) FROM secrets", [], |r| r.get(0))
            .expect("secrets query");
        assert_eq!(secrets, 0);
        let runs: i64 = conn
            .query_row("SELECT COUNT(*) FROM cron_runs", [], |r| r.get(0))
            .expect("cron_runs survived");
        assert_eq!(runs, 1);

        // v8 additive columns landed on the pre-existing row with defaults.
        let (fired_by, attempt): (String, i64) = conn
            .query_row(
                "SELECT fired_by, attempt FROM cron_runs WHERE id='r1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("v8 columns");
        assert_eq!(fired_by, "cron");
        assert_eq!(attempt, 1);
    }

    #[test]
    fn fts5_table_exists_after_migration() {
        let conn = in_memory();
        run(&conn).expect("migrations");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='messages_fts'",
                [],
                |r| r.get(0),
            )
            .expect("query");
        assert_eq!(count, 1, "messages_fts virtual table should exist");
    }

    #[test]
    fn fts5_inserts_are_searchable() {
        let conn = in_memory();
        run(&conn).expect("migrations");

        conn.execute("INSERT INTO sessions (id) VALUES ('s1')", [])
            .expect("session");
        conn.execute(
            "INSERT INTO messages (session_id, role, content) VALUES ('s1', 'user', 'hello world')",
            [],
        )
        .expect("message");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'hello'",
                [],
                |r| r.get(0),
            )
            .expect("fts query");
        assert_eq!(count, 1);
    }

    #[test]
    fn fts5_porter_stemmer_matches_stemmed_word() {
        let conn = in_memory();
        run(&conn).expect("migrations");

        conn.execute("INSERT INTO sessions (id) VALUES ('s2')", [])
            .expect("session");
        conn.execute(
            "INSERT INTO messages (session_id, role, content) VALUES ('s2', 'assistant', 'running fast')",
            [],
        ).expect("message");

        // 'run' should match 'running' via porter stemmer
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'run'",
                [],
                |r| r.get(0),
            )
            .expect("fts query");
        assert_eq!(count, 1, "porter stemmer should match 'running' via 'run'");
    }

    #[test]
    fn messages_fk_enforced() {
        let conn = in_memory();
        run(&conn).expect("migrations");

        // Inserting a message with a non-existent session_id should fail due to FK.
        let result = conn.execute(
            "INSERT INTO messages (session_id, role, content) VALUES ('no-such-session', 'user', 'oops')",
            [],
        );
        assert!(
            result.is_err(),
            "FK constraint should reject orphaned message"
        );
    }
}
