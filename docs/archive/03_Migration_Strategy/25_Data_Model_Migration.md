# Data Model Migration

> **Status:** ✅ Complete
> **Category:** Migration Strategy

---

## 1. OpenClaw → Talon Types

### Session

**OpenClaw (TypeScript):**
```typescript
interface Session {
  id: string;
  title?: string;
  messages: Message[];
  createdAt: Date;
  updatedAt: Date;
  metadata?: Record<string, unknown>;
}
```

**Talon (Rust):**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub title: Option<String>,
    pub source: SessionSource,
    pub profile: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
// Messages stored separately in DB, loaded on demand
```

Key changes:
- `string` id → `Uuid` (enforced at type level)
- `Date` → `DateTime<Utc>` (timezone-explicit)
- `messages` removed from struct — loaded from SQLite when needed (lazy)
- `metadata` → explicit `source` + `profile` fields

---

### Message

**OpenClaw:**
```typescript
interface Message {
  role: "user" | "assistant" | "system" | "tool";
  content: string | ContentBlock[];
  toolCallId?: string;
  name?: string;
}
```

**Hermes (Python):**
```python
class Message(BaseModel):
    role: Literal["user", "assistant", "system", "tool"]
    content: Union[str, List[ContentBlock]]
    tool_call_id: Optional[str] = None
    name: Optional[str] = None
```

**Talon (Rust):**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: MessageContent,
    pub tool_call_id: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role { User, Assistant, System, Tool }
```

---

### Tool Call

**OpenClaw:**
```typescript
interface ToolCall {
  id: string;
  type: "function";
  function: {
    name: string;
    arguments: string;  // JSON string!
  };
}
```

**Talon:**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,  // parsed, not string
}

impl ToolCall {
    /// Parse from OpenAI wire format (arguments is a JSON string)
    pub fn from_openai(raw: &OpenAiToolCall) -> Result<Self, serde_json::Error> {
        Ok(Self {
            id: raw.id.clone(),
            name: raw.function.name.clone(),
            arguments: serde_json::from_str(&raw.function.arguments)?,
        })
    }
}
```

---

### Skill

**Hermes (Python dict):**
```python
{
  "name": "github-pr-workflow",
  "path": "/home/user/.hermes/skills/github-pr-workflow/SKILL.md",
  "category": "github",
  "description": "GitHub PR lifecycle...",
  "pinned": False,
}
```

**Talon:**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub path: PathBuf,
    pub category: Option<String>,
    pub description: String,
    pub pinned: bool,
    pub updated_at: DateTime<Utc>,
    // Cached content (loaded on first skill_view)
    #[serde(skip)]
    pub content_cache: Option<String>,
}

/// YAML frontmatter parsed from SKILL.md
#[derive(Debug, Deserialize)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    pub category: Option<String>,
    pub pinned: Option<bool>,
    pub tags: Option<Vec<String>>,
}
```

---

### Cron Job

**Hermes (Python):**
```python
@dataclass
class CronJob:
    job_id: str
    name: Optional[str]
    schedule: str
    prompt: str
    enabled: bool = True
    deliver: str = "origin"
    skills: list[str] = field(default_factory=list)
    context_from: list[str] = field(default_factory=list)
```

**Talon:**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: Uuid,
    pub name: Option<String>,
    pub schedule: CronSchedule,
    pub prompt: String,
    pub enabled: bool,
    pub deliver_to: DeliverTarget,
    pub skills: Vec<String>,
    pub context_from: Vec<Uuid>,
    pub repeat: Option<u32>,
    pub no_agent: bool,
    pub script: Option<PathBuf>,
    pub created_at: DateTime<Utc>,
    pub last_run: Option<DateTime<Utc>>,
    pub run_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeliverTarget {
    Origin,
    Local,
    All,
    Platform { platform: String, chat_id: String, thread_id: Option<String> },
}
```

---

## 2. SQLite Serialization Strategy

All complex types stored as JSON columns:

```rust
// Storing a Vec<ContentBlock> in SQLite
let content_json = serde_json::to_string(&message.content)?;
conn.execute(
    "INSERT INTO messages(session_id, role, content) VALUES (?1, ?2, ?3)",
    params![session_id.to_string(), role_str, content_json],
)?;

// Reading back
let content_json: String = row.get(2)?;
let content: MessageContent = serde_json::from_str(&content_json)?;
```

---

## 3. Migration: Hermes SQLite → Talon SQLite

```rust
pub async fn migrate_from_hermes(
    hermes_db: &Path,
    talon_db: &Path,
) -> Result<MigrationReport, MigrationError> {
    let src = rusqlite::Connection::open(hermes_db)?;
    let dst = rusqlite::Connection::open(talon_db)?;
    apply_migrations(&dst)?;

    // Migrate sessions
    let sessions: Vec<(String, String, i64)> = src
        .prepare("SELECT id, source, created_at FROM sessions")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;

    for (id, source, ts) in &sessions {
        dst.execute(
            "INSERT OR IGNORE INTO sessions(id, source, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)",
            params![id, source, ts],
        )?;
    }

    // Migrate messages (content may need schema upgrade)
    // ...

    Ok(MigrationReport {
        sessions_migrated: sessions.len(),
        // ...
    })
}
```

---

## 4. Config Format Migration

**OpenClaw `.env` + `config.json`:**
```json
{
  "defaultModel": "gpt-4o",
  "maxTokens": 4096,
  "approvalMode": "ask_for_dangerous"
}
```

**Talon `[config.toml](../02_Architecture/18a_Config_System.md)`:**
```toml
[agent]
model = "claude-sonnet-4-5"
max_tokens = 8192
approval_level = "ask_for_dangerous"
max_iterations = 50

[llm.providers.anthropic]
type = "anthropic"
api_key = "${ANTHROPIC_API_KEY}"

[memory]
db_path = "~/.talon/talon.db"
profile_dir = "~/.talon"

[logging]
level = "info"
json = false
```

Migration script converts `.env` + `config.json` → `config.toml` automatically.
---

## Related Documents

### See Also
- [Canonical Types](../00_Connections/05_Canonical_Types.md)
- [SQLite & FTS5 in Rust](../07_Memory_System/55_SQLite_FTS5_In_Rust.md)
- [Context & Memory Architecture](../02_Architecture/15_Context_And_Memory_Architecture.md)

