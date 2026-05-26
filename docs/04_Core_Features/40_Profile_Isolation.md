# Profile Isolation

> **Status:** ✅ Complete
> **Category:** Core Features

---

## 1. What are Profiles?

A **profile** is a complete isolated Talon environment with its own:
- SQLite database (sessions, memory, cron jobs)
- Skill files
- Config overrides
- System prompt
- User profile notes

Profiles enable multi-persona or multi-project deployments on a single host.

---

## 2. Directory Structure

```
~/.talon/
├── config.toml          ← default profile config
├── talon.db            ← default profile database
├── skills/              ← default profile skills
├── memories/            ← default profile memory notes
└── profiles/
    ├── work/
    │   ├── config.toml
    │   ├── talon.db
    │   ├── skills/
    │   └── memories/
    └── research/
        ├── config.toml
        ├── talon.db
        ├── skills/
        └── memories/
```

The **default profile** lives directly in `~/.talon/`.
Named profiles live in `~/.talon/profiles/<name>/`.

---

## 3. Profile Struct

```rust
// talon-core/src/profile.rs

#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub base_dir: PathBuf,
}

impl Profile {
    /// Load the default profile
    pub fn default() -> Result<Self, ProfileError> {
        let base = dirs::home_dir()
            .ok_or(ProfileError::NoHomeDir)?
            .join(".talon");
        Ok(Self { name: "default".to_string(), base_dir: base })
    }

    /// Load a named profile
    pub fn named(name: &str) -> Result<Self, ProfileError> {
        let base = dirs::home_dir()
            .ok_or(ProfileError::NoHomeDir)?
            .join(".talon")
            .join("profiles")
            .join(name);

        if !base.exists() {
            return Err(ProfileError::NotFound(name.to_string()));
        }

        Ok(Self { name: name.to_string(), base_dir: base })
    }

    /// Create a new named profile
    pub async fn create(name: &str) -> Result<Self, ProfileError> {
        let base = dirs::home_dir()
            .ok_or(ProfileError::NoHomeDir)?
            .join(".talon")
            .join("profiles")
            .join(name);

        if base.exists() {
            return Err(ProfileError::AlreadyExists(name.to_string()));
        }

        tokio::fs::create_dir_all(&base).await?;
        tokio::fs::create_dir_all(base.join("skills")).await?;
        tokio::fs::create_dir_all(base.join("memories")).await?;

        Ok(Self { name: name.to_string(), base_dir: base })
    }

    pub fn db_path(&self) -> PathBuf     { self.base_dir.join("talon.db") }
    pub fn config_path(&self) -> PathBuf { self.base_dir.join("config.toml") }
    pub fn skills_dir(&self) -> PathBuf  { self.base_dir.join("skills") }
    pub fn memories_dir(&self) -> PathBuf { self.base_dir.join("memories") }
    pub fn user_profile_path(&self) -> PathBuf {
        self.memories_dir().join("USER.md")
    }
    pub fn agent_memory_path(&self) -> PathBuf {
        self.memories_dir().join("MEMORY.md")
    }
}
```

---

## 4. Profile Loading at Startup

```rust
pub async fn initialize_from_cli(cli: &Cli) -> Result<AppState, AppError> {
    // Resolve profile
    let profile = match &cli.profile {
        Some(name) if name == "default" => Profile::default()?,
        Some(name) => Profile::named(name)?,
        None => {
            // Check env var
            match std::env::var("TALON_PROFILE").ok() {
                Some(name) => Profile::named(&name)?,
                None => Profile::default()?,
            }
        }
    };

    tracing::info!(profile = profile.name, "Loading profile");

    // Load config (profile-specific overrides global)
    let mut config = load_config(cli.config.as_deref())?;
    if profile.config_path().exists() {
        let profile_config = load_config(Some(
            profile.config_path().to_str().unwrap()
        ))?;
        config.merge(profile_config);
    }

    // Initialize DB for this profile
    let memory = MemoryStore::open(profile.db_path()).await?;
    memory.migrate().await?;

    // Load skills from profile's skill dir
    let skills = SkillStore::open(profile.skills_dir()).await?;

    Ok(AppState { profile, config, memory: Arc::new(memory), skills: Arc::new(skills) })
}
```

---

## 5. Profile Isolation for Cron Jobs

Cron jobs are profile-scoped. When a [cron job](33_Cron_Scheduler.md) is scheduled under profile `work`,
it only runs in that profile's context and cannot access `research` profile data.

```rust
pub struct CronJob {
    // ...
    pub profile: Option<String>,  // None = same as scheduling profile
}

impl CronRunner {
    async fn run_job(&self, job: &CronJob) -> CronRunResult {
        // Load the profile for this job
        let profile = match &job.profile {
            Some(name) => Profile::named(name)?,
            None => self.current_profile.clone(),
        };

        // Create isolated app state for this run
        let state = AppState::for_profile(profile).await?;

        // Run in complete isolation from other profiles
        let mut agent = AgentLoop::from_state(state);
        agent.run(vec![Message::user(job.prompt.clone())], Uuid::new_v4()).await
    }
}
```

---

## 6. CLI Profile Commands

```bash
# List all profiles
talon profiles list
# → default (active)
# → work
# → research

# Create a new profile
talon profiles create work

# Switch active profile (sets TALON_PROFILE for the session)
talon profiles use work

# Delete a profile (requires confirmation)
talon profiles delete research --confirm

# Show current profile
talon profiles current
# → work

# Run a one-off command in a different profile
talon --profile research chat "What was I working on last week?"
```

---

## 7. Cross-Profile Safety

Talon **never** reads another profile's data unless explicitly asked.
The `memory` tool only writes to the currently-active profile.
Skills loaded from the active profile's `skills/` dir never
automatically see the global default skills.

Cross-profile reads are possible but explicit:

```bash
# Copy a skill from one profile to another
talon profiles copy-skill work::docker-deployment research
```

This is intentionally verbose — profile isolation means isolation.
---

## Related Documents

### Depends On
- [Config System](../02_Architecture/18a_Config_System.md)

### See Also
- [Memory System](35_Memory_System_SQLite_FTS5.md)
- [File System Tool](31_File_System_Tool.md)
- [Cron Scheduler](33_Cron_Scheduler.md)

