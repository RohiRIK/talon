# hello — example Talon skill

The smallest end-to-end skill: imports the host `log` capability and exports
`run`. Given `{"name": "<who>"}` it logs a line and returns `"Hello, <who>!"`.

## Layout

| File | Role |
|------|------|
| `wit/world.wit` | The `talon:skill` contract. **Must stay byte-identical** to `crates/talon-plugins/wit/world.wit`. |
| `src/lib.rs` | Guest implementation (wit-bindgen). |
| `hello.toml` | Host-trusted sidecar manifest: name, description, `approval_level`, capability allowlist, input schema. |
| `build.sh` | Compiles to a `wasm32-wasip2` component and refreshes the test fixture. |

## Building

```sh
./build.sh
```

This writes `crates/talon-plugins/tests/fixtures/hello.wasm`, the committed
fixture the `talon-plugins` integration tests load. **Re-run it whenever
`wit/world.wit` or `src/lib.rs` changes.**

### Toolchain gotcha

This crate targets `wasm32-wasip2`, whose std lives only in the **rustup**-managed
`stable` toolchain. The main workspace builds on **Homebrew** rust, which has no
wasm std and is first on `PATH` — so a plain `cargo build --target wasm32-wasip2`
fails with *"can't find crate for core/std"*. `build.sh` works around this by
resolving the rustup rustc (`rustup which --toolchain stable rustc`) and pinning
`RUSTC` to it for the build only. Prerequisites:

```sh
brew install rustup && rustup-init -y
rustup target add wasm32-wasip2
```

## Deploying

Drop both files into the skills directory; Talon hot-reloads them:

```sh
cp target/wasm32-wasip2/release/hello_skill.wasm ~/.talon/skills/hello.wasm
cp hello.toml ~/.talon/skills/hello.toml
```
