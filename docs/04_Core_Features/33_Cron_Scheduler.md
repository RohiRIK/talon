# Cron Scheduler & Job Management

> **Status:** ✅ Complete
> **Category:** Core Features

---

## 1. Architecture

```
┌─────────────────────────────────────────────────────┐
│               CronScheduler                         │
│                                                     │
│  ┌───────────┐   ┌────────────┐   ┌──────────────┐ │
│  │ tokio-    │   │  CronStore │   │  JobRunner   │ │
│  │ cron-sched│──→│  (SQLite)  │   │  (per-job    │ │
│  │           │   │            │   │   Tokio task)│ │
│  └─────┬─────┘   └──────┬─────┘   └──────┬───────┘ │
│        │                │                │         │
│        └────────────────┴────────────────┘         │
│                         │                          │
│              ┌──────────▼──────────┐               │
│              │  Agent Session      │               │
│              │  (runs prompt as    │               │
│              │   ephemeral session)│               │
│              └─────────────────────┘               │
└─────────────────────────────────────────────────────┘
```

---

## 2. CronJob Type

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: Uuid,
    pub name: Option<String>,
    pub schedule: CronSchedule,
    pub prompt: String,
    pub enabled: bool,
    pub deliver_to: DeliverTarget,
    pub skills: Vec<String>,
    pub context_from: Vec<Uuid>,  // job IDs whose output to inject
    pub created_at: DateTime<Utc>,
    pub last_run: Option<DateTime<Utc>>,
    pub last_output: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CronSchedule {
    /// Standard cron expression: "0 9 * * *"
    Cron(String),
    /// Human interval: "30m", "every 2h", "daily"
    Human(String),
    /// One-shot ISO timestamp
    Once(DateTime<Utc>),
}

impl CronSchedule {
    pub fn to_cron_str(&self) -> Result<String, SchedulerError> {
        match self {
            Self::Cron(s) => Ok(s.clone()),
            Self::Human(s) => parse_human_interval(s),
            Self::Once(dt) => Ok(format!(
                "{} {} {} {} *",
                dt.minute(), dt.hour(), dt.day(), dt.month()
            )),
        }
    }
}

fn parse_human_interval(s: &str) -> Result<String, SchedulerError> {
    let s = s.to_lowercase();
    let s = s.trim_start_matches("every ");

    if let Some(n) = s.strip_suffix('m').and_then(|n| n.parse::<u32>().ok()) {
        return Ok(format!("*/{n} * * * *"));
    }
    if let Some(n) = s.strip_suffix('h').and_then(|n| n.parse::<u32>().ok()) {
        return Ok(format!("0 */{n} * * *"));
    }
    match s {
        "daily"   => Ok("0 9 * * *".into()),
        "hourly"  => Ok("0 * * * *".into()),
        "weekly"  => Ok("0 9 * * 1".into()),
        "monthly" => Ok("0 9 1 * *".into()),
        _ => Err(SchedulerError::UnknownInterval(s.into())),
    }
}
```

---

## 3. CronStore (SQLite)

```rust
impl CronStore {
    pub async fn upsert(&self, job: &CronJob) -> Result<(), MemoryError> {
        let pool = self.pool.clone();
        let job = job.clone();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get()?;
            conn.execute("
                INSERT OR REPLACE INTO cron_jobs
                    (id, name, schedule, prompt, enabled, deliver_to, created_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ", rusqlite::params![
                job.id.to_string(),
                job.name,
                serde_json::to_string(&job.schedule)?,
                job.prompt,
                job.enabled as i32,
                serde_json::to_string(&job.deliver_to)?,
                job.created_at.timestamp(),
            ])?;
            Ok::<_, MemoryError>(())
        }).await?
    }

    pub async fn list_enabled(&self) -> Result<Vec<CronJob>, MemoryError> { /* ... */ }
    pub async fn update_last_run(&self, id: Uuid, output: &str) -> Result<(), MemoryError> { /* ... */ }
}
```

---

## 4. Scheduler Bootstrap

```rust
pub struct CronScheduler {
    inner: JobScheduler,
    store: Arc<CronStore>,
    agent_factory: Arc<dyn AgentFactory>,
    gateway: Arc<GatewayRouter>,
}

impl CronScheduler {
    pub async fn start(&self) -> Result<(), SchedulerError> {
        self.inner.start().await?;
        let jobs = self.store.list_enabled().await?;
        for job in jobs {
            self.register_job(job).await?;
        }
        Ok(())
    }

    async fn register_job(&self, job: CronJob) -> Result<(), SchedulerError> {
        let cron_str = job.schedule.to_cron_str()?;
        let store = self.store.clone();
        let agent_factory = self.agent_factory.clone();
        let gateway = self.gateway.clone();
        let job_clone = job.clone();

        let sched_job = Job::new_async(cron_str.as_str(), move |_uuid, _lock| {
            let store = store.clone();
            let agent_factory = agent_factory.clone();
            let gateway = gateway.clone();
            let job = job_clone.clone();
            Box::pin(async move {
                run_cron_job(job, store, agent_factory, gateway).await
                    .unwrap_or_else(|e| tracing::error!(error=%e, "cron job failed"));
            })
        })?;

        self.inner.add(sched_job).await?;
        Ok(())
    }
}
```

---

## 5. Job Execution

```rust
async fn run_cron_job(
    job: CronJob,
    store: Arc<CronStore>,
    agent_factory: Arc<dyn AgentFactory>,
    gateway: Arc<GatewayRouter>,
) -> Result<(), SchedulerError> {
    tracing::info!(job_id=%job.id, name=?job.name, "cron job firing");

    // Inject context from upstream jobs
    let context = build_job_context(&job, &store).await?;

    // Build full prompt
    let prompt = if context.is_empty() {
        job.prompt.clone()
    } else {
        format!("## Context from upstream jobs:\n{context}\n\n## Task:\n{}", job.prompt)
    };

    // Create ephemeral agent session
    let agent = agent_factory.create_ephemeral(&job).await?;
    let session_id = Uuid::new_v4();

    // Run agent
    let output = agent.run_single_turn(session_id, &prompt).await?;

    // Persist output
    store.update_last_run(job.id, &output).await?;

    // Deliver to target
    gateway.deliver(&job.deliver_to, &output).await?;

    Ok(())
}
```

---

## 6. `cronjob` Tool (LLM-Facing)

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CronjobParams {
    /// "create" | "list" | "update" | "pause" | "resume" | "remove" | "run"
    pub action: String,
    pub job_id: Option<String>,
    pub name: Option<String>,
    /// "30m", "every 2h", "0 9 * * *", or ISO timestamp
    pub schedule: Option<String>,
    pub prompt: Option<String>,
    pub enabled_toolsets: Option<Vec<String>>,
    pub deliver: Option<String>,
}
```

---

## 7. Repeat Count & One-Shot Jobs

```rust
// In CronJob
pub struct CronJob {
    // ...
    pub repeat: Option<u32>,    // None = infinite, Some(1) = one-shot
    pub run_count: u32,
}

// In run_cron_job()
job.run_count += 1;
if let Some(max) = job.repeat {
    if job.run_count >= max {
        store.pause(job.id).await?;
        inner.remove(&sched_uuid).await?;
        tracing::info!(job_id=%job.id, "one-shot job complete, removed from scheduler");
    }
}
```
---

## Related Documents

### Depends On
- [State Machine & Lifecycle](../02_Architecture/14_State_Machine_And_Lifecycle.md)
- [Core Agent Loop Design](../02_Architecture/13_Core_Agent_Loop_Design.md)

### See Also
- [Tokio Runtime Design](../06_Concurrency/49_Tokio_Runtime_Design.md)
- [Profile Isolation](40_Profile_Isolation.md)
- [Gateway Architecture](../02_Architecture/18_Gateway_MultiChannel_Architecture.md)

