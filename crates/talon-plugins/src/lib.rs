//! Talon WASM plugin host (Phase 6).
//!
//! A *skill* is a WebAssembly **component** (WASI preview2) that implements the
//! `talon:skill` world in `wit/world.wit`: it may import host capabilities and
//! exports `run(input) -> result<string, string>`. The host loads skills from
//! `~/.talon/skills/`, hot-reloads them, gates host calls by a sidecar manifest,
//! and exposes each as an `Arc<dyn Tool>`.
//!
//! The whole subsystem is behind the opt-in `wasm` feature — it pulls wasmtime
//! (15–22 MB) plus WASI and a filesystem watcher.

pub mod manifest;

#[cfg(feature = "wasm")]
pub mod host;

#[cfg(feature = "wasm")]
pub mod store;

#[cfg(feature = "wasm")]
pub use host::PluginHost;

#[cfg(feature = "wasm")]
pub use store::SkillStore;
