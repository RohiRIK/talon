# TypeScript-to-Rust Migration Patterns

> **Last corrected:** dogfood pass 4

> **Status:** ✅ Complete
> **Category:** Migration Strategy

> **Cross-reference note (dogfood pass 2):** No `[See doc XX_...]` style
> links were found in this file — no broken cross-references to repair.
> If future edits add cross-references, verify target files exist before
> committing.

---

## 1. Async/Await & Promises

**TypeScript:**
```typescript
async function fetchUser(id: string): Promise<User> {
  const res = await fetch(`/api/users/${id}`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}
```

**Rust:**
```rust
async fn fetch_user(id: &str) -> Result<User, reqwest::Error> {
    let url = format!("/api/users/{id}");
    let user: User = reqwest::get(&url).await?.json().await?;
    Ok(user)
}
```

Key difference: Rust `?` propagates errors, no `try/catch`. Return type is explicit.

---

## 2. Optional Chaining & Null Safety

**TypeScript:**
```typescript
const name = user?.profile?.displayName ?? "Anonymous";
```

**Rust:**
```rust
let name = user
    .and_then(|u| u.profile)
    .and_then(|p| p.display_name)
    .unwrap_or("Anonymous");
```

Or with `Option::map_or`:
```rust
let name = user.as_ref()
    .and_then(|u| u.profile.as_ref())
    .map(|p| p.display_name.as_str())
    .unwrap_or("Anonymous");
```

---

## 3. Union Types → Enums

**TypeScript:**
```typescript
type ContentBlock =
  | { type: "text"; text: string }
  | { type: "image"; url: string; alt?: string }
  | { type: "tool_use"; id: string; name: string; input: unknown };
```

**Rust:**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    Image { url: String, alt: Option<String> },
    ToolUse { id: String, name: String, input: serde_json::Value },
}
```

---

## 4. Interface → Trait

**TypeScript:**
```typescript
interface LlmProvider {
  complete(req: CompletionRequest): AsyncIterable<Delta>;
  supportsVision(): boolean;
}
```

**Rust:**
```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(
        &self,
        req: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Delta, LlmError>> + Send>>, LlmError>;

    fn supports_vision(&self) -> bool { false }
}
```

---

## 5. EventEmitter → Tokio Broadcast

**TypeScript:**
```typescript
emitter.emit("tool_complete", { id, output });
emitter.on("tool_complete", handler);
```

**Rust:**
```rust
// Sender (broadcast channel)
event_tx.send(AgentEvent::ToolCallCompleted {
    call_id: id,
    output: output,
}).ok();

// Subscriber
let mut rx = event_tx.subscribe();
while let Ok(event) = rx.recv().await {
    match event {
        AgentEvent::ToolCallCompleted { call_id, output } => { /* ... */ }
        _ => {}
    }
}
```

---

## 6. Map/Filter/Reduce → Iterators

**TypeScript:**
```typescript
const names = tools
  .filter(t => t.enabled)
  .map(t => t.name)
  .sort();
```

**Rust:**
```rust
let mut names: Vec<&str> = tools.iter()
    .filter(|t| t.enabled)
    .map(|t| t.name.as_str())
    .collect();
names.sort();
```

---

## 7. Dynamic Objects → serde_json::Value

**TypeScript:**
```typescript
const args: Record<string, unknown> = JSON.parse(rawJson);
const cmd = args["command"] as string;
```

**Rust:**
```rust
let args: serde_json::Value = serde_json::from_str(raw_json)?;
let cmd = args["command"]
    .as_str()
    .ok_or(ToolError::MissingArg("command".into()))?;
```

---

## 8. Class Inheritance → Trait + Struct Composition

**TypeScript:**
```typescript
class BaseGateway {
  protected abstract send(msg: OutboundMessage): Promise<void>;
  async broadcast(msgs: OutboundMessage[]) {
    await Promise.all(msgs.map(m => this.send(m)));
  }
}
class TelegramGateway extends BaseGateway { ... }
```

**Rust:**
```rust
// Default method on trait = "inheritance"
#[async_trait]
pub trait Gateway: Send + Sync {
    async fn send(&self, msg: OutboundMessage) -> Result<(), GatewayError>;

    async fn broadcast(&self, msgs: Vec<OutboundMessage>) -> Result<(), GatewayError> {
        let futs = msgs.into_iter().map(|m| self.send(m));
        futures::future::try_join_all(futs).await.map(|_| ())
    }
}

pub struct TelegramGateway { /* ... */ }
#[async_trait]
impl Gateway for TelegramGateway {
    async fn send(&self, msg: OutboundMessage) -> Result<(), GatewayError> { /* ... */ }
}
```

---

## 9. setTimeout / Interval → Tokio Timers

**TypeScript:**
```typescript
const id = setInterval(() => runJob(job), intervalMs);
clearInterval(id);
```

**Rust:**
```rust
let mut interval = tokio::time::interval(Duration::from_millis(interval_ms));
loop {
    interval.tick().await;
    if shutdown_rx.try_recv().is_ok() { break; }
    run_job(&job).await?;
}
```

---

## 10. try/catch → Result + ?

**TypeScript:**
```typescript
try {
  const data = await readFile(path);
  return JSON.parse(data);
} catch (e) {
  console.error("Failed:", e);
  return null;
}
```

**Rust:**
```rust
async fn load_json(path: &Path) -> Option<serde_json::Value> {
    let data = tokio::fs::read_to_string(path).await.ok()?;
    serde_json::from_str(&data).ok()
}
```

Or with explicit error propagation:
```rust
async fn load_json(path: &Path) -> Result<serde_json::Value, AgentError> {
    let data = tokio::fs::read_to_string(path).await
        .map_err(|e| AgentError::IoError { path: path.into(), source: e })?;
    serde_json::from_str(&data)
        .map_err(|e| AgentError::ParseError(e.to_string()))
}
```
---

## Related Documents

### Depends On
- [TypeScript Pain Points](../01_Analysis/07_TypeScript_Pain_Points.md)

### See Also
- [Async Migration (Node→Tokio)](24_Async_Migration_NodeJS_To_Tokio.md)
- [Data Model Migration](25_Data_Model_Migration.md)

