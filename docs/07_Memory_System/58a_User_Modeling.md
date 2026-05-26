# User Modeling

> **Status:** ✅ Complete
> **Category:** Memory System

---

## 1. Overview

Talon maintains a USER.md file containing stable facts about the user.
This is one of the two persistent memory files (the other being MEMORY.md).

The distinction:
- **USER.md**: who the user IS (name, role, preferences, communication style, pet peeves)
- **MEMORY.md**: what Talon has LEARNED (environment facts, project conventions, tool quirks)

---

## 2. USER.md Format

```markdown
# User Profile

Name: Rohi Rikman
Role: Freelance developer / agency owner
GitHub: RohiRIK
Timezone: Asia/Jerusalem
Language: English (Hebrew in speech; English in CLI/code)
Communication style: Direct, terse. Skip preambles.
Stack: Bun/Next.js, Rust, Docker, Supabase

## Preferences
- Self-hosted over SaaS
- Fix root causes, not workarounds
- Always ask before modifying package.json or config files
- Use `bun` not npm; `uv` not pip

## Pet Peeves
- Over-explanation of basics
- Applying "fixes" without asking
- Hebrew in terminal output (renders garbled)
```

---

## 3. USER.md Injection

USER.md is injected into the system prompt inside `<user_profile>` XML tags:

```
<user_profile>
Name: Rohi Rikman
Role: Freelance developer...
[full USER.md content]
</user_profile>
```

The LLM reads this before processing every request, allowing it to:
- Use the correct name
- Apply the correct communication style
- Avoid known pet peeves
- Make correct tool choices (bun vs npm)

---

## 4. User Model Update Flow

The LLM updates USER.md via the `memory` tool:

```json
{
  "tool": "memory",
  "parameters": {
    "action": "add",
    "target": "user",
    "content": "User prefers dark-themed SVG diagrams over light ones."
  }
}
```

```rust
pub async fn update_user_profile(
    profile_dir: &Path,
    action: MemoryAction,
    target: MemoryTarget,
    content: &str,
    old_text: Option<&str>,
) -> Result<(), MemoryError> {
    let path = match target {
        MemoryTarget::User => profile_dir.join("memories/USER.md"),
        MemoryTarget::Memory => profile_dir.join("memories/MEMORY.md"),
    };

    let existing = if path.exists() {
        tokio::fs::read_to_string(&path).await?
    } else {
        String::new()
    };

    let updated = match action {
        MemoryAction::Add => {
            format!("{}\n{}", existing.trim_end(), content)
        }
        MemoryAction::Replace => {
            let old = old_text.ok_or(MemoryError::MissingOldText)?;
            if !existing.contains(old) {
                return Err(MemoryError::OldTextNotFound(old.to_string()));
            }
            existing.replacen(old, content, 1)
        }
        MemoryAction::Remove => {
            let old = old_text.ok_or(MemoryError::MissingOldText)?;
            existing.replace(old, "")
        }
    };

    tokio::fs::write(&path, updated.trim_end().to_string() + "\n").await?;
    Ok(())
}
```

---

## 5. When Should Talon Update USER.md?

Trigger conditions:
1. User explicitly says "remember this" or "don't do that again"
2. User corrects a wrong assumption Talon made
3. User reveals a stable preference, habit, or personal detail
4. Talon discovers a convention specific to this user's workflow

Anti-patterns (do NOT save):
- Task progress or session outcomes (ephemeral)
- PR/issue/commit numbers (stale within a week)
- Any fact that changes per-project

---

## 6. Memory Size Guardrails

```rust
const USER_MD_SOFT_LIMIT: usize = 1_375;
const MEMORY_MD_SOFT_LIMIT: usize = 2_200;

pub fn check_memory_size(path: &Path, content: &str, limit: usize) -> Option<String> {
    let pct = (content.len() * 100) / limit;
    if pct >= 90 {
        Some(format!(
            "⚠️  {} is at {}% capacity ({}/{} chars). \
             Consider consolidating entries to stay under the limit.",
            path.display(), pct, content.len(), limit
        ))
    } else {
        None
    }
}
```

When over 90%, Talon warns the user and offers to consolidate.
When over 100%, Talon refuses to add more and demands consolidation first.

---

## 7. Mem0 Integration (Optional)

For users who want vector-backed semantic memory, Talon optionally
integrates with a self-hosted Mem0 instance (Qdrant + Ollama backend):

```rust
pub struct Mem0Client {
    base_url: String,
    user_id: String,
    client: reqwest::Client,
}

impl Mem0Client {
    pub async fn search(&self, query: &str, top_k: u32) -> Result<Vec<MemoryHit>> {
        let resp: SearchResponse = self.client
            .post(format!("{}/v1/memories/search/", self.base_url))
            .json(&json!({
                "query": query,
                "user_id": self.user_id,
                "limit": top_k
            }))
            .send().await?
            .json().await?;
        Ok(resp.results)
    }

    pub async fn add(&self, content: &str) -> Result<()> {
        self.client
            .post(format!("{}/v1/memories/", self.base_url))
            .json(&json!({
                "messages": [{"role": "user", "content": content}],
                "user_id": self.user_id
            }))
            .send().await?;
        Ok(())
    }
}
```

When enabled, Mem0 supplements USER.md/MEMORY.md with semantic retrieval,
surfacing relevant past facts that wouldn't fit in the context window.
---

## Related Documents

### Depends On
- [Context & Memory Architecture](../02_Architecture/15_Context_And_Memory_Architecture.md)

### See Also
- [Session Management](56_Session_Management.md)
- [Memory System](../04_Core_Features/35_Memory_System_SQLite_FTS5.md)

