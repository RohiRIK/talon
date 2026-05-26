# Python-to-Rust Migration Patterns

> **Status:** ✅ Complete
> **Category:** Migration Strategy
> **Last corrected:** dogfood pass 3

---

## 1. Dataclasses → Structs + Serde

**Python:**
```python
from dataclasses import dataclass, field
from typing import Optional

@dataclass
class Session:
    id: str
    title: Optional[str] = None
    messages: list[dict] = field(default_factory=list)
```

**Rust:**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub title: Option<String>,
    #[serde(default)]
    pub messages: Vec<Message>,
}
```

---

## 2. Pydantic Validators → serde + custom Deserialize

**Python:**
```python
from pydantic import BaseModel, validator

class ToolCall(BaseModel):
    name: str
    arguments: dict

    @validator("name")
    def name_must_be_alphanumeric(cls, v):
        assert v.isidentifier(), "name must be identifier"
        return v
```

**Rust:**
```rust
#[derive(Debug, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

impl ToolCall {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if !self.name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(ValidationError::InvalidToolName(self.name.clone()));
        }
        Ok(())
    }
}
```

---

## 3. asyncio.gather → futures::join_all

**Python:**
```python
results = await asyncio.gather(
    tool_a.execute(args_a),
    tool_b.execute(args_b),
    return_exceptions=True,
)
```

**Rust:**
```rust
let results = futures::future::join_all(vec![
    tool_a.execute(args_a),
    tool_b.execute(args_b),
]).await;
// Each result is Result<ToolResult, ToolError>
```

---

## 4. subprocess.run → tokio::process::Command

**Python:**
```python
result = subprocess.run(
    ["rg", "--json", pattern, path],
    capture_output=True,
    text=True,
    timeout=30,
)
```

**Rust:**
```rust
let output = tokio::process::Command::new("rg")
    .args(["--json", pattern, path])
    .output()
    .await?;

let stdout = String::from_utf8_lossy(&output.stdout);
```

With timeout:
```rust
let output = tokio::time::timeout(
    Duration::from_secs(30),
    tokio::process::Command::new("rg").args([...]).output(),
).await
.map_err(|_| ToolError::Timeout)??;
```

---

## 5. Dict Comprehension → .map().collect()

**Python:**
```python
schema = {tool.name: tool.description for tool in tools}
```

**Rust:**
```rust
let schema: HashMap<&str, &str> = tools.iter()
    .map(|t| (t.name(), t.description()))
    .collect();
```

---

## 6. Context Managers → RAII + Drop

**Python:**
```python
with open(path, "w") as f:
    f.write(content)

async with aiofiles.open(path, "w") as f:
    await f.write(content)
```

**Rust:**
```rust
// File is closed when it goes out of scope (Drop)
let mut f = tokio::fs::File::create(path).await?;
f.write_all(content.as_bytes()).await?;
// f dropped here, file closed
```

Or via `tokio::fs::write` for the one-liner:
```rust
tokio::fs::write(path, content).await?;
```

---

## 7. Exception Hierarchy → thiserror

**Python:**
```python
class AgentError(Exception): pass
class ToolError(AgentError): pass
class LlmError(AgentError): pass
class RateLimitError(LlmError):
    def __init__(self, retry_after: int):
        self.retry_after = retry_after
```

**Rust:**
```rust
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("rate limited, retry after {retry_after}s")]
    RateLimit { retry_after: u64 },

    #[error("API error {status}: {message}")]
    ApiError { status: u16, message: String },

    #[error(transparent)]
    Network(#[from] reqwest::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error(transparent)]
    Llm(#[from] LlmError),
    #[error(transparent)]
    Tool(#[from] ToolError),
}
```

---

## 8. Generators / yield → async Stream

**Python:**
```python
async def stream_response(prompt: str):
    async for chunk in client.stream(prompt):
        yield chunk.delta
```

**Rust:**
```rust
use futures::stream::{self, Stream};
use async_stream::stream;

fn stream_response(prompt: String) -> impl Stream<Item = Result<Delta, LlmError>> {
    stream! {
        let mut resp = client.stream(&prompt).await?;
        while let Some(chunk) = resp.next().await {
            yield chunk?.delta;
        }
    }
}
```

---

## 9. SQLite (sqlite3 / aiosqlite) → rusqlite

**Python:**
```python
async with aiosqlite.connect(path) as db:
    await db.execute(
        "INSERT INTO messages(session_id, role, content) VALUES (?, ?, ?)",
        (session_id, role, content)
    )
    await db.commit()
```

**Rust (with connection pool via r2d2):**
```rust
conn.execute(
    "INSERT INTO messages(session_id, role, content) VALUES (?1, ?2, ?3)",
    params![session_id.to_string(), role, content],
)?;
```

Note: [rusqlite](../07_Memory_System/55_SQLite_FTS5_In_Rust.md) is synchronous — wrap in `tokio::task::spawn_blocking` for async contexts:
```rust
let conn = pool.get()?;
tokio::task::spawn_blocking(move || {
    conn.execute("INSERT ...", params![...])?;
    Ok::<_, rusqlite::Error>(())
}).await??;
```

---

## 10. @lru_cache / functools → once_cell / cached

**Python:**
```python
from functools import lru_cache

@lru_cache(maxsize=128)
def load_skill(name: str) -> Skill: ...
```

**Rust:**
```rust
use std::sync::OnceLock;
use std::collections::HashMap;

static SKILL_CACHE: OnceLock<Arc<Mutex<HashMap<String, Skill>>>> = OnceLock::new();

fn get_cache() -> &'static Arc<Mutex<HashMap<String, Skill>>> {
    SKILL_CACHE.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}
```

Or use the `cached` crate for `#[cached]` attribute macro.
---

## Related Documents

### Depends On
- [Python Pain Points](../01_Analysis/08_Python_Pain_Points.md)

### See Also
- [Data Model Migration](25_Data_Model_Migration.md)
- [Error Handling Strategy](../06_Concurrency/54_Error_Handling_Strategy.md)

