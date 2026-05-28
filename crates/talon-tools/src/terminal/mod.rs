pub mod docker;
pub mod native;

pub use docker::DockerSandbox;
pub use native::NativeBackend;

use std::{future::Future, pin::Pin};

use talon_core::tools::ToolResult;

/// The active sandbox mode — determines isolation level and approval behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMode {
    Docker,
    Native,
}

/// Abstraction over sandbox execution backends.
pub trait SandboxBackend: Send + Sync {
    fn mode(&self) -> SandboxMode;
    fn execute(&self, cmd: &str) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>>;
}
