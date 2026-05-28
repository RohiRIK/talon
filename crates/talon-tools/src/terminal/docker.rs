use std::{future::Future, pin::Pin};

use talon_core::tools::ToolResult;

use super::{SandboxBackend, SandboxMode};

/// Embedded seccomp profile — blocks dangerous syscalls at the kernel level.
const SECCOMP_PROFILE: &str = include_str!("seccomp.json");

/// Runs commands inside a disposable Docker container.
/// Network is disabled, memory capped at 512 MB, seccomp profile applied.
/// `rm -rf /` and other destructive syscalls are blocked by the kernel.
pub struct DockerSandbox {
    image: String,
}

impl DockerSandbox {
    pub fn new(image: impl Into<String>) -> Self {
        Self {
            image: image.into(),
        }
    }
}

impl Default for DockerSandbox {
    fn default() -> Self {
        Self::new("talon-sandbox:latest")
    }
}

impl SandboxBackend for DockerSandbox {
    fn mode(&self) -> SandboxMode {
        SandboxMode::Docker
    }

    fn execute(&self, cmd: &str) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        let image = self.image.clone();
        let cmd = cmd.to_string();
        Box::pin(async move {
            // Write seccomp profile to a temp file so Docker can read it.
            let tmp = match tempfile::Builder::new().suffix(".json").tempfile() {
                Ok(f) => f,
                Err(e) => return ToolResult::err(format!("cannot create seccomp tmpfile: {e}")),
            };

            if let Err(e) = tokio::fs::write(tmp.path(), SECCOMP_PROFILE).await {
                return ToolResult::err(format!("cannot write seccomp profile: {e}"));
            }

            let seccomp_path = tmp.path().to_string_lossy().into_owned();

            let output = tokio::process::Command::new("docker")
                .args([
                    "run",
                    "--rm",
                    "--network=none",
                    "--memory=512m",
                    "--memory-swap=512m",
                    &format!("--security-opt=seccomp={seccomp_path}"),
                    "--user=nobody",
                    &image,
                    "sh",
                    "-c",
                    &cmd,
                ])
                .output()
                .await;

            // tmp lives until here — Docker has already read the file by now.
            drop(tmp);

            match output {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let combined = if stderr.is_empty() {
                        stdout.into_owned()
                    } else {
                        format!("{stdout}{stderr}")
                    };
                    if out.status.success() {
                        ToolResult::ok(combined)
                    } else {
                        ToolResult::err(format!(
                            "exit {}: {combined}",
                            out.status.code().unwrap_or(-1)
                        ))
                    }
                }
                Err(e) => ToolResult::err(format!("docker not available: {e}")),
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn mode_is_docker() {
        assert_eq!(DockerSandbox::default().mode(), SandboxMode::Docker);
    }

    #[test]
    fn default_image_is_talon_sandbox() {
        assert_eq!(DockerSandbox::default().image, "talon-sandbox:latest");
    }

    #[test]
    fn custom_image_name_is_preserved() {
        let s = DockerSandbox::new("my-image:v1");
        assert_eq!(s.image, "my-image:v1");
    }

    #[test]
    fn seccomp_profile_is_valid_json() {
        let _: serde_json::Value =
            serde_json::from_str(SECCOMP_PROFILE).expect("seccomp.json must be valid JSON");
    }

    /// Live Docker smoke test — skipped by default.
    /// Run with: cargo nextest run --run-ignored all -E 'test(docker_smoke)'
    #[tokio::test]
    #[ignore = "requires Docker daemon"]
    async fn docker_smoke_echo_succeeds() {
        let s = DockerSandbox::new("alpine:latest");
        let r = s.execute("echo sandbox_ok").await;
        assert!(!r.is_error, "{}", r.content);
        assert!(r.content.contains("sandbox_ok"));
    }
}
