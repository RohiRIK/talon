//! Skill store: load compiled `.wasm` skills (+ their host-trusted sidecar
//! manifests) from a directory and hot-reload them when the directory changes.
//!
//! A skill on disk is a pair: `<name>.wasm` (the component) and `<name>.toml`
//! (the [`SkillManifest`]). Either one missing or malformed skips that skill with
//! a warning — one bad skill never takes down the rest of the store.
//!
//! [`SkillStore::watch`] installs a [`notify`] watcher that re-scans the whole
//! directory on any change, so dropping a new `.wasm` in makes it appear without
//! restarting Talon (Phase 6 exit gate: tool list updates within 2s).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use wasmtime::component::Component;

use crate::host::{PluginHost, SkillRun};
use crate::manifest::SkillManifest;

/// A skill that compiled and parsed cleanly: its manifest plus the ready-to-run
/// component. [`Component`] is cheap to clone (it is `Arc`-backed internally).
#[derive(Clone)]
struct LoadedSkill {
    manifest: SkillManifest,
    component: Component,
}

/// Holds every skill loaded from `dir`, keyed by manifest name. Cloneable handle
/// semantics: the skill map is shared (`Arc`), so the background watcher and any
/// caller see the same live set.
pub struct SkillStore {
    host: Arc<PluginHost>,
    dir: PathBuf,
    skills: Arc<RwLock<HashMap<String, LoadedSkill>>>,
    _watcher: Option<RecommendedWatcher>,
}

impl SkillStore {
    /// Create a store over `dir` and load whatever is already there. Does not
    /// watch — call [`SkillStore::watch`] for hot-reload.
    pub fn new(host: Arc<PluginHost>, dir: PathBuf) -> Self {
        let skills = Arc::new(RwLock::new(scan(&host, &dir)));
        Self {
            host,
            dir,
            skills,
            _watcher: None,
        }
    }

    /// Re-scan the directory and replace the loaded set.
    pub fn reload(&self) {
        let fresh = scan(&self.host, &self.dir);
        if let Ok(mut map) = self.skills.write() {
            *map = fresh;
        }
    }

    /// Install a filesystem watcher: any change under `dir` triggers a full
    /// re-scan. The watcher is owned by the store; dropping the store stops it.
    pub fn watch(&mut self) -> anyhow::Result<()> {
        let host = self.host.clone();
        let dir = self.dir.clone();
        let skills = self.skills.clone();
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
                Ok(_) => {
                    let fresh = scan(&host, &dir);
                    if let Ok(mut map) = skills.write() {
                        *map = fresh;
                    }
                }
                Err(e) => tracing::warn!(target: "talon_skill", "skill watcher error: {e}"),
            })
            .map_err(|e| anyhow::anyhow!("creating skill directory watcher: {e}"))?;
        watcher
            .watch(&self.dir, RecursiveMode::NonRecursive)
            .map_err(|e| anyhow::anyhow!("watching skill directory {}: {e}", self.dir.display()))?;
        self._watcher = Some(watcher);
        Ok(())
    }

    /// Names of all currently loaded skills.
    pub fn names(&self) -> Vec<String> {
        self.skills
            .read()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// The manifest of a loaded skill, if present.
    pub fn manifest(&self, name: &str) -> Option<SkillManifest> {
        self.skills
            .read()
            .ok()
            .and_then(|m| m.get(name).map(|s| s.manifest.clone()))
    }

    /// Run a loaded skill by name, enforcing its manifest capability allowlist.
    pub fn run(&self, name: &str, input: &str) -> anyhow::Result<SkillRun> {
        let skill = self
            .skills
            .read()
            .ok()
            .and_then(|m| m.get(name).cloned())
            .ok_or_else(|| anyhow::anyhow!("no loaded skill named '{name}'"))?;
        self.host
            .run(&skill.component, skill.manifest.capabilities.clone(), input)
    }
}

/// Scan `dir` for `<name>.wasm` + `<name>.toml` pairs, compiling and parsing
/// each. Any skill that fails to read, parse, or compile is skipped with a
/// warning so a single bad file cannot break the whole store. Keyed by the
/// manifest's `name`, not the filename — the manifest is the source of truth.
fn scan(host: &PluginHost, dir: &Path) -> HashMap<String, LoadedSkill> {
    let mut out = HashMap::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
            continue;
        }
        let toml_path = path.with_extension("toml");
        let manifest = match SkillManifest::load(&toml_path) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(target: "talon_skill", "skipping {}: {e}", path.display());
                continue;
            }
        };
        let component = match host.compile_file(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(target: "talon_skill", "skipping {}: {e}", path.display());
                continue;
            }
        };
        out.insert(
            manifest.name.clone(),
            LoadedSkill {
                manifest,
                component,
            },
        );
    }
    out
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn host() -> Arc<PluginHost> {
        Arc::new(PluginHost::new().expect("host"))
    }

    #[test]
    fn empty_dir_has_no_skills() {
        let dir = tempfile::tempdir().expect("tmp");
        let store = SkillStore::new(host(), dir.path().to_path_buf());
        assert!(store.names().is_empty());
    }

    #[test]
    fn wasm_without_sidecar_is_skipped() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("orphan.wasm"), b"\0asm").expect("write");
        let store = SkillStore::new(host(), dir.path().to_path_buf());
        assert!(store.names().is_empty(), "no sidecar manifest => skipped");
    }

    #[test]
    fn invalid_wasm_with_valid_sidecar_is_skipped() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("bad.wasm"), b"not a component").expect("write");
        std::fs::write(
            dir.path().join("bad.toml"),
            "name = \"bad\"\ncapabilities = [\"log\"]\n",
        )
        .expect("write");
        let store = SkillStore::new(host(), dir.path().to_path_buf());
        assert!(store.names().is_empty(), "garbage wasm => skipped");
    }

    #[test]
    fn reload_picks_up_filesystem_state() {
        let dir = tempfile::tempdir().expect("tmp");
        let store = SkillStore::new(host(), dir.path().to_path_buf());
        assert!(store.names().is_empty());
        std::fs::write(dir.path().join("x.wasm"), b"garbage").expect("write");
        store.reload();
        // still empty: garbage doesn't compile, but reload ran without panicking
        assert!(store.names().is_empty());
    }

    #[test]
    fn run_unknown_skill_errors() {
        let dir = tempfile::tempdir().expect("tmp");
        let store = SkillStore::new(host(), dir.path().to_path_buf());
        assert!(store.run("nope", "{}").is_err());
    }
}
