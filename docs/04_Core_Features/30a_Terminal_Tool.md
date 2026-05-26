# Terminal Tool Implementation

> **Status:** ✅ Complete
> **Category:** Core Features
> **Last corrected:** dogfood pass 3

---

## 1. Overview

The terminal tool executes shell commands in a sandboxed environment.
It is the highest-risk tool in the system — every design decision prioritizes safety.

```
User/LLM Request
      │
      ▼
ApprovalMembrane (risk = Destructive)
      │ approved
      ▼
SandboxBackend::execute(cmd, opts)
      │
      ├── Docker backend   (production)
      ├── Direct backend   (dev/tests only)
      └── Mock backend     (tests)
      │
      ▼
TerminalOutput { stdout, stderr, exit_code, duration }
```

---

## 2. Tool Trait Implementation

```rust
use crate::tools::{Tool, ToolContext, ToolResult, ToolError};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use async_trait::async_trait;

#[derive(Debug, Default)]
pub struct TerminalTool {
    backend: Arc<dyn TerminalBackend>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TerminalParams {
    /// Shell command to execute
    pub command: String,

    /// Timeout in seconds (default: 180, max: 600)
    #[serde(default = "default_timeout")]
    #[schemars(range(min = 1, max = 600))]
    pub timeout: u64,

    /// Working directory (absolute path)
    pub workdir: Option<String>,

    /// Run in background and return session ID
    #[serde(default)]
    pub background: bool,

    /// Notify on completion (only used when background=true)
    #[serde(default)]
    pub notify_on_complete: bool,
}

fn default_timeout() -> u64 { 180 }

#[async_trait]
impl Tool for TerminalTool {
    fn name(&self) -> &str { "terminal" }

    fn description(&self) -> &str {
        "Execute shell commands. Reserve for builds, installs, git, processes, \
         and network operations. Do not use for file reads/writes — use read_file \
         and write_file instead."
    }

    fn parameters(&self) -> schemars::schema::RootSchema {
        schema_for!(TerminalParams)
    }

    fn risk_level(&self) -> ToolRisk { ToolRisk::Destructive }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let params: TerminalParams = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidParams(e.to_string()))?;

        if params.background {
            return self.execute_background(params, ctx).await;
        }

        let result = self.backend
            .execute(ExecuteRequest {
                command: params.command,
                timeout: Duration::from_secs(params.timeout),
                workdir: params.workdir.map(PathBuf::from),
                session_id: ctx.session_id,
            })
            .await
            .map_err(ToolError::Execution)?;

        Ok(ToolResult::text(format!(
            "exit_code: {}\n\n{}{}",
            result.exit_code,
            result.stdout,
            if result.stderr.is_empty() { String::new() }
            else { format!("\nstderr:\n{}", result.stderr) }
        )))
    }
}
```

---

## 3. Sandbox Backend Trait

```rust
#[async_trait]
pub trait TerminalBackend: Send + Sync {
    async fn execute(&self, req: ExecuteRequest) -> Result<TerminalResult, SandboxError>;

    async fn spawn_background(
        &self,
        req: ExecuteRequest,
    ) -> Result<BackgroundSession, SandboxError>;

    async fn poll_session(&self, session_id: Uuid) -> Result<SessionStatus, SandboxError>;
    async fn kill_session(&self, session_id: Uuid) -> Result<(), SandboxError>;
    async fn send_stdin(&self, session_id: Uuid, data: &str) -> Result<(), SandboxError>;
    async fn session_log(&self, session_id: Uuid, offset: u64) -> Result<Vec<String>, SandboxError>;
}

#[derive(Debug)]
pub struct ExecuteRequest {
    pub command: String,
    pub timeout: Duration,
    pub workdir: Option<PathBuf>,
    pub session_id: Uuid,
}

#[derive(Debug)]
pub struct TerminalResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration: Duration,
    pub truncated: bool,
}
```

---

## 4. Docker Sandbox Backend

```rust
pub struct DockerSandboxBackend {
    image: String,          // "ghcr.io/yourorg/talon-sandbox:latest"
    network: SandboxNetwork,
    mem_limit: String,      // "512m"
    cpu_quota: i64,         // 50000 = 50% of one CPU
    seccomp_profile: String,
    allowed_mounts: Vec<AllowedMount>,
}

#[async_trait]
impl TerminalBackend for DockerSandboxBackend {
    async fn execute(&self, req: ExecuteRequest) -> Result<TerminalResult, SandboxError> {
        let container_id = self.create_container(&req).await?;

        let output = tokio::time::timeout(
            req.timeout + Duration::from_secs(5), // grace period
            self.wait_container(&container_id),
        )
        .await
        .map_err(|_| SandboxError::Timeout)?
        .map_err(SandboxError::Docker)?;

        self.remove_container(&container_id).await.ok();

        Ok(TerminalResult {
            stdout: truncate_output(output.stdout, MAX_OUTPUT_BYTES),
            stderr: truncate_output(output.stderr, MAX_OUTPUT_BYTES),
            exit_code: output.exit_code,
            duration: output.duration,
            truncated: output.stdout.len() >= MAX_OUTPUT_BYTES
                || output.stderr.len() >= MAX_OUTPUT_BYTES,
        })
    }
}

impl DockerSandboxBackend {
    async fn create_container(
        &self,
        req: &ExecuteRequest,
    ) -> Result<String, SandboxError> {
        // Build docker run args
        let mut args = vec![
            "run".to_string(),
            "--rm".into(),
            "-d".into(),
            "--network".into(), self.network.to_string(),
            "--memory".into(), self.mem_limit.clone(),
            "--cpu-quota".into(), self.cpu_quota.to_string(),
            "--security-opt".into(), format!("seccomp={}", self.seccomp_profile),
            "--read-only".into(),     // immutable root filesystem
            "--tmpfs".into(), "/tmp:rw,noexec,nosuid,size=64m".into(),
        ];

        // Mount project directory read-only
        for mount in &self.allowed_mounts {
            args.extend([
                "-v".into(),
                format!("{}:{}:{}", mount.host_path.display(),
                        mount.container_path.display(),
                        if mount.writable { "rw" } else { "ro" }),
            ]);
        }

        if let Some(workdir) = &req.workdir {
            args.extend(["-w".into(), workdir.to_string_lossy().to_string()]);
        }

        args.push(self.image.clone());
        args.extend(["sh".into(), "-c".into(), req.command.clone()]);

        let output = tokio::process::Command::new("docker")
            .args(&args)
            .output()
            .await
            .map_err(|e| SandboxError::Spawn(e.to_string()))?;

        if !output.status.success() {
            return Err(SandboxError::Docker(
                String::from_utf8_lossy(&output.stderr).into_owned()
            ));
        }

        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }
}
```

---

## 5. Direct Backend (dev / tests)

```rust
pub struct DirectBackend {
    allowed_workdirs: Vec<PathBuf>,
}

#[async_trait]
impl TerminalBackend for DirectBackend {
    async fn execute(&self, req: ExecuteRequest) -> Result<TerminalResult, SandboxError> {
        // Validate workdir is in allowed list
        if let Some(ref wd) = req.workdir {
            if !self.is_allowed(wd) {
                return Err(SandboxError::ForbiddenPath(wd.clone()));
            }
        }

        let start = std::time::Instant::now();
        let result = tokio::time::timeout(
            req.timeout,
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&req.command)
                .current_dir(req.workdir.unwrap_or_else(|| PathBuf::from(".")))
                .output(),
        )
        .await
        .map_err(|_| SandboxError::Timeout)?
        .map_err(|e| SandboxError::Spawn(e.to_string()))?;

        Ok(TerminalResult {
            stdout: String::from_utf8_lossy(&result.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
            exit_code: result.status.code().unwrap_or(-1),
            duration: start.elapsed(),
            truncated: false,
        })
    }
}
```

---

## 6. Background Process Management

```rust
pub struct ProcessStore {
    sessions: RwLock<HashMap<Uuid, BackgroundProcess>>,
}

struct BackgroundProcess {
    id: Uuid,
    command: String,
    started_at: DateTime<Utc>,
    child: tokio::process::Child,
    stdout_buf: Arc<Mutex<RingBuffer<String>>>,
    stderr_buf: Arc<Mutex<RingBuffer<String>>>,
    status: ProcessStatus,
    notify_tx: Option<oneshot::Sender<ProcessStatus>>,
}

impl ProcessStore {
    pub async fn spawn(
        &self,
        req: ExecuteRequest,
        notify_on_complete: bool,
    ) -> Result<BackgroundSession, SandboxError> {
        let id = Uuid::new_v4();

        let mut child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&req.command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| SandboxError::Spawn(e.to_string()))?;

        // Drain stdout into ring buffer
        let stdout_buf = Arc::new(Mutex::new(RingBuffer::new(1000)));
        tokio::spawn(drain_reader(
            child.stdout.take().unwrap(),
            stdout_buf.clone(),
        ));

        // Store
        let mut sessions = self.sessions.write().await;
        sessions.insert(id, BackgroundProcess {
            id, command: req.command, started_at: Utc::now(),
            child, stdout_buf, status: ProcessStatus::Running,
            notify_tx: None,
        });

        Ok(BackgroundSession { id, started_at: Utc::now() })
    }
}
```

---

## 7. Output Formatting

```rust
const MAX_OUTPUT_BYTES: usize = 50 * 1024;  // 50KB

fn truncate_output(s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    let half = max / 2;
    format!(
        "{}\n\n... [truncated {} bytes] ...\n\n{}",
        &s[..half],
        s.len() - max,
        &s[s.len() - half..]
    )
}
```
---

## Related Documents

### Depends On
- [Tool System Architecture](../02_Architecture/16_Tool_System_Architecture.md)
- [Approval Membrane](../02_Architecture/17a_Approval_Membrane.md)

### See Also
- [Security Model](../02_Architecture/20_Security_Model.md)
- [Docker & Container Deployment](../08_DevOps/61_Docker_And_Container_Deployment.md)
- [Tool Execution Engine](30_Tool_Execution_Engine.md)

