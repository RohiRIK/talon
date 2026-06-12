//! Webhook registrations (migration v8, criteria 25–27).
//!
//! Stores only the *name* of the signing secret — the secret itself lives in
//! the builtin vault. Revocation is a tombstone; the public delivery path
//! resolves active hooks only.

use std::sync::Arc;

use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::{Database, error::MemoryError};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Webhook {
    pub id: String,
    pub job_id: String,
    /// Builtin-vault key holding the HMAC signing secret. Never serialized
    /// to API responses with the secret value — this is just the name.
    pub secret_name: String,
    pub created_at: String,
    pub revoked: bool,
}

fn now_utc() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[derive(Clone)]
pub struct WebhookStore {
    db: Arc<Database>,
}

impl WebhookStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Register a hook for a job. The caller stores the signing secret in
    /// the vault under the returned `secret_name`.
    pub async fn create(&self, job_id: &str) -> Result<Webhook, MemoryError> {
        let hook = Webhook {
            id: Uuid::new_v4().simple().to_string(),
            job_id: job_id.to_string(),
            secret_name: String::new(),
            created_at: now_utc(),
            revoked: false,
        };
        let hook = Webhook {
            secret_name: format!("webhook/{}", hook.id),
            ..hook
        };

        let row = hook.clone();
        self.interact(move |conn| {
            conn.execute(
                "INSERT INTO webhooks (id, job_id, secret_name, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![row.id, row.job_id, row.secret_name, row.created_at],
            )
            .map(|_| ())
        })
        .await?;
        Ok(hook)
    }

    /// Active (non-revoked) hook by id — the delivery path's lookup.
    pub async fn get_active(&self, id: &str) -> Result<Option<Webhook>, MemoryError> {
        let id = id.to_string();
        self.interact(move |conn| {
            conn.query_row(
                "SELECT id, job_id, secret_name, created_at FROM webhooks
                  WHERE id = ?1 AND revoked_at IS NULL",
                params![id],
                |row| {
                    Ok(Webhook {
                        id: row.get(0)?,
                        job_id: row.get(1)?,
                        secret_name: row.get(2)?,
                        created_at: row.get(3)?,
                        revoked: false,
                    })
                },
            )
            .optional()
        })
        .await
    }

    pub async fn list_for_job(&self, job_id: &str) -> Result<Vec<Webhook>, MemoryError> {
        let job_id = job_id.to_string();
        self.interact(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, job_id, secret_name, created_at, revoked_at
                   FROM webhooks WHERE job_id = ?1 ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map(params![job_id], |row| {
                Ok(Webhook {
                    id: row.get(0)?,
                    job_id: row.get(1)?,
                    secret_name: row.get(2)?,
                    created_at: row.get(3)?,
                    revoked: row.get::<_, Option<String>>(4)?.is_some(),
                })
            })?;
            rows.collect()
        })
        .await
    }

    /// Tombstone a hook; `Ok(true)` when an active hook was revoked.
    pub async fn revoke(&self, id: &str) -> Result<bool, MemoryError> {
        let id = id.to_string();
        let now = now_utc();
        self.interact(move |conn| {
            conn.execute(
                "UPDATE webhooks SET revoked_at = ?1 WHERE id = ?2 AND revoked_at IS NULL",
                params![now, id],
            )
            .map(|n| n > 0)
        })
        .await
    }

    async fn interact<T, F>(&self, f: F) -> Result<T, MemoryError>
    where
        T: Send + 'static,
        F: FnOnce(&mut rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let conn = self.db.pool().get().await?;
        Ok(conn.interact(move |conn| f(conn)).await??)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{CronJob, CronSchedule, CronStore};

    async fn setup() -> (CronStore, WebhookStore, String) {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        let cron = CronStore::new(Arc::clone(&db));
        let job = cron
            .create(CronJob::new(
                "p",
                CronSchedule::Cron("0 0 * * *".into()),
                "s",
            ))
            .await
            .expect("job");
        (cron, WebhookStore::new(db), job.id)
    }

    #[tokio::test]
    async fn create_get_revoke_roundtrip() {
        let (_cron, hooks, job_id) = setup().await;
        let hook = hooks.create(&job_id).await.expect("create");
        assert_eq!(hook.secret_name, format!("webhook/{}", hook.id));

        let active = hooks
            .get_active(&hook.id)
            .await
            .expect("get")
            .expect("found");
        assert_eq!(active.job_id, job_id);

        assert!(hooks.revoke(&hook.id).await.expect("revoke"));
        assert!(hooks.get_active(&hook.id).await.expect("get").is_none());
        assert!(!hooks.revoke(&hook.id).await.expect("idempotent"));

        // Tombstone still listed for the job.
        let listed = hooks.list_for_job(&job_id).await.expect("list");
        assert_eq!(listed.len(), 1);
        assert!(listed[0].revoked);
    }

    #[tokio::test]
    async fn deleting_job_cascades_hooks() {
        let (cron, hooks, job_id) = setup().await;
        let hook = hooks.create(&job_id).await.expect("create");
        cron.delete(&job_id).await.expect("delete job");
        assert!(hooks.get_active(&hook.id).await.expect("get").is_none());
    }
}
