//! The `hello` example skill — the smallest end-to-end Talon skill.
//!
//! It imports the host `log` capability and exports `run`: given a JSON string
//! `{"name": "..."}`, it logs a line through the host and returns a greeting.
//! Build with `wasm32-wasip2`, which emits a WASI 0.2 component directly.

wit_bindgen::generate!({
    world: "skill",
    path: "wit",
});

use talon::skill::host::log;

struct HelloSkill;

impl Guest for HelloSkill {
    fn run(input: String) -> Result<String, String> {
        let name = serde_json_name(&input).unwrap_or_else(|| "world".to_string());
        log(&format!("hello skill greeting {name}"));
        Ok(format!("Hello, {name}!"))
    }
}

/// Tiny hand-rolled extractor for `{"name": "<value>"}` so the example needs no
/// JSON dependency. Returns the first string value of a top-level `name` key.
fn serde_json_name(input: &str) -> Option<String> {
    let key = input.find("\"name\"")?;
    let after = &input[key + "\"name\"".len()..];
    let colon = after.find(':')?;
    let rest = after[colon + 1..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

export!(HelloSkill);
