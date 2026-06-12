//! wasmtime component host: compile a skill component, link WASI preview2 + the
//! `talon:skill` host capabilities, instantiate it in a fresh sandbox, and call
//! its exported `run`.
//!
//! Each `run` gets its own [`Store`] with a default-deny [`WasiCtx`] (no
//! filesystem, env, or network) — a skill reaches the outside world only through
//! the host capabilities its manifest grants (see [`crate::manifest`]).

use std::path::Path;

use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

#[allow(clippy::all, missing_docs)]
mod bindings {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "skill",
        imports: { default: trappable },
    });
}

use bindings::Skill;
use bindings::talon::skill::host::Host;

/// Per-instance store state: the WASI context + table, the capability allowlist
/// this skill was granted (enforced in [`Host::log`]), and captured log lines.
struct SkillState {
    wasi: WasiCtx,
    table: ResourceTable,
    granted: Vec<String>,
    logs: Vec<String>,
}

impl WasiView for SkillState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl Host for SkillState {
    /// Host `log` capability. Gated: if the skill's manifest did not grant `log`,
    /// the call traps (returns `Err`), failing the guest rather than silently
    /// allowing an undeclared capability. This is the [`crate::sandbox`] seam.
    fn log(&mut self, msg: String) -> wasmtime::Result<()> {
        if !self.granted.iter().any(|c| c == "log") {
            return Err(wasmtime::Error::msg(
                "skill called host.log without the 'log' capability",
            ));
        }
        tracing::debug!(target: "talon_skill", "{msg}");
        self.logs.push(msg);
        Ok(())
    }
}

/// The result of running a skill: the guest's `result<string, string>` plus any
/// host-log lines it emitted during the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRun {
    pub output: Result<String, String>,
    pub logs: Vec<String>,
}

/// Owns the shared wasmtime [`Engine`] and a pre-populated component [`Linker`]
/// (WASI + host capabilities). Cheap to clone components against; one host
/// serves every loaded skill.
pub struct PluginHost {
    engine: Engine,
    linker: Linker<SkillState>,
}

impl PluginHost {
    pub fn new() -> anyhow::Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        let engine = Engine::new(&config)?;

        let mut linker = Linker::<SkillState>::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
            .map_err(|e| anyhow::anyhow!("linking WASI preview2 into the component linker: {e}"))?;
        Skill::add_to_linker::<_, HasSelf<_>>(&mut linker, |s| s)
            .map_err(|e| anyhow::anyhow!("linking talon:skill host capabilities: {e}"))?;

        Ok(Self { engine, linker })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Compile a `.wasm` component from disk. Compilation is the expensive step;
    /// callers cache the returned [`Component`] and reuse it across runs.
    pub fn compile_file(&self, path: &Path) -> anyhow::Result<Component> {
        Component::from_file(&self.engine, path)
            .map_err(|e| anyhow::anyhow!("compiling skill component {}: {e}", path.display()))
    }

    /// Compile a component from raw bytes (used in tests / embedded fixtures).
    pub fn compile(&self, bytes: &[u8]) -> anyhow::Result<Component> {
        Component::from_binary(&self.engine, bytes)
            .map_err(|e| anyhow::anyhow!("compiling skill component from bytes: {e}"))
    }

    /// Instantiate `component` in a fresh default-deny sandbox and call its
    /// exported `run(input)`. `granted` is the manifest capability allowlist.
    pub fn run(
        &self,
        component: &Component,
        granted: Vec<String>,
        input: &str,
    ) -> anyhow::Result<SkillRun> {
        let mut store = Store::new(
            &self.engine,
            SkillState {
                wasi: WasiCtxBuilder::new().build(),
                table: ResourceTable::new(),
                granted,
                logs: Vec::new(),
            },
        );
        let bindings = Skill::instantiate(&mut store, component, &self.linker)
            .map_err(|e| anyhow::anyhow!("instantiating skill component: {e}"))?;
        let output = bindings
            .call_run(&mut store, input)
            .map_err(|e| anyhow::anyhow!("calling skill export run: {e}"))?;
        Ok(SkillRun {
            output,
            logs: std::mem::take(&mut store.data_mut().logs),
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn host_constructs_with_wasi_and_capabilities_linked() {
        // Building the host wires WASI preview2 + the talon:skill host import
        // into one linker; a failure here means a missing/renamed import.
        PluginHost::new().expect("host builds with WASI + host caps linked");
    }

    #[test]
    fn rejects_non_component_bytes() {
        let host = PluginHost::new().expect("host");
        let result = host.compile(b"not a wasm component");
        assert!(result.is_err(), "garbage bytes must not compile");
    }
}
