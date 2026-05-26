# File System Tools Implementation

> **Status:** ✅ Complete
> **Category:** Core Features
> **Last corrected:** dogfood pass 3

---

## 1. Tool Suite Overview

Four distinct tools cover filesystem operations:

| Tool | Risk | Description |
|------|------|-------------|
| `read_file` | ReadOnly | Paginated line-numbered file reading |
| `write_file` | Destructive | Full file overwrite (creates dirs) |
| `patch` | Destructive | Targeted find-and-replace edits |
| `search_files` | ReadOnly | Content grep or filename glob via ripgrep |

Each is a separate struct implementing `Tool`. They share a `FileSystemContext`
that enforces path restrictions.

---

## 2. Path Security

```rust
pub struct FileSystemContext {
    /// Absolute paths the agent is allowed to touch
    allowed_roots: Vec<PathBuf>,
    /// Hard block list — never touch these regardless of roots
    blocked_paths: Vec<PathBuf>,
}

impl FileSystemContext {
    pub fn validate(&self, path: &Path) -> Result<PathBuf, ToolError> {
        // Canonicalize (resolves symlinks, .., etc.)
        let canonical = path
            .canonicalize()
            .or_else(|_| {
                // File may not exist yet (write_file) — canonicalize parent
                let parent = path.parent().ok_or(ToolError::InvalidPath)?;
                parent.canonicalize().map(|p| p.join(path.file_name().ok_or(ToolError::InvalidPath)?))
            })
            .map_err(|_| ToolError::InvalidPath)?;

        // Check block list first
        for blocked in &self.blocked_paths {
            if canonical.starts_with(blocked) {
                return Err(ToolError::PathBlocked(canonical));
            }
        }

        // Must be under at least one allowed root
        let allowed = self.allowed_roots.iter().any(|r| canonical.starts_with(r));
        if !allowed {
            return Err(ToolError::PathNotAllowed(canonical));
        }

        Ok(canonical)
    }
}
```

Default `blocked_paths`:
- `~/.ssh`
- `~/.gnupg`
- `/etc/passwd`, `/etc/shadow`
- `~/.talon/memories/` (only writable via memory tool)

---

## 3. ReadFile Tool

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadFileParams {
    /// Absolute or relative path to the file
    pub path: String,
    /// Line to start reading from (1-indexed, default: 1)
    #[serde(default = "one")]
    pub offset: usize,
    /// Max lines to return (default: 500, max: 2000)
    #[serde(default = "default_read_limit")]
    #[schemars(range(min = 1, max = 2000))]
    pub limit: usize,
}

fn one() -> usize { 1 }
fn default_read_limit() -> usize { 500 }

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str { "read_file" }
    fn risk_level(&self) -> ToolRisk { ToolRisk::ReadOnly }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let p: ReadFileParams = serde_json::from_value(args)?;
        let path = ctx.fs.validate(Path::new(&p.path))?;

        // Reject binary files
        if is_binary(&path).await? {
            return Err(ToolError::BinaryFile(path));
        }

        let content = tokio::fs::read_to_string(&path).await
            .map_err(ToolError::Io)?;

        let total_lines = content.lines().count();
        let start = (p.offset - 1).min(total_lines);
        let end   = (start + p.limit).min(total_lines);

        let numbered: String = content
            .lines()
            .enumerate()
            .skip(start)
            .take(end - start)
            .map(|(i, line)| format!("{}|{}", i + 1, line))
            .collect::<Vec<_>>()
            .join("\n");

        Ok(ToolResult::text(format!(
            start + 1, end, total_lines
        )))
    }
}

async fn is_binary(path: &Path) -> Result<bool, ToolError> {
    use tokio::io::AsyncReadExt;
    let mut f = tokio::fs::File::open(path).await.map_err(ToolError::Io)?;
    let mut buf = [0u8; 8192];
    let n = f.read(&mut buf).await.map_err(ToolError::Io)?;
    // Null byte heuristic
    Ok(buf[..n].contains(&0u8))
}
```

---

## 4. WriteFile Tool

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteFileParams {
    pub path: String,
    /// Complete file content to write
    pub content: String,
    /// Skip syntax check (default: false)
    #[serde(default)]
    pub skip_lint: bool,
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str { "write_file" }
    fn risk_level(&self) -> ToolRisk { ToolRisk::Destructive }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let p: WriteFileParams = serde_json::from_value(args)?;
        let path = ctx.fs.validate(Path::new(&p.path))?;

        // Create parent dirs
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(ToolError::Io)?;
        }

        tokio::fs::write(&path, &p.content).await.map_err(ToolError::Io)?;

        // Run syntax check for known languages
        let lint_result = if !p.skip_lint {
            lint_file(&path).await
        } else {
            None
        };

        let line_count = p.content.lines().count();
        let mut out = format!("Written: {} ({} lines)", path.display(), line_count);

        if let Some(lint) = lint_result {
            if !lint.errors.is_empty() {
                out.push_str(&format!("\n\n⚠️ Lint errors:\n{}", lint.errors.join("\n")));
            }
        }

        Ok(ToolResult::text(out))
    }
}

async fn lint_file(path: &Path) -> Option<LintResult> {
    match path.extension()?.to_str()? {
        "rs"   => run_lint("rustfmt", &["--check", &path.to_string_lossy()]).await,
        "py"   => run_lint("ruff", &["check", &path.to_string_lossy()]).await,
        "ts" | "js" => run_lint("biome", &["check", &path.to_string_lossy()]).await,
        "json" => {
            // Parse with serde — zero external deps
            let content = tokio::fs::read_to_string(path).await.ok()?;
            match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(_) => None,
                Err(e) => Some(LintResult { errors: vec![e.to_string()] }),
            }
        }
        _ => None,
    }
}
```

---

## 5. Patch Tool

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PatchParams {
    pub path: String,
    /// Exact text to find (must be unique unless replace_all=true)
    pub old_string: String,
    /// Replacement text
    pub new_string: String,
    /// Replace all occurrences (default: false)
    #[serde(default)]
    pub replace_all: bool,
}

#[async_trait]
impl Tool for PatchTool {
    fn name(&self) -> &str { "patch" }
    fn risk_level(&self) -> ToolRisk { ToolRisk::Destructive }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let p: PatchParams = serde_json::from_value(args)?;
        let path = ctx.fs.validate(Path::new(&p.path))?;

        let content = tokio::fs::read_to_string(&path).await.map_err(ToolError::Io)?;

        let count = content.matches(p.old_string.as_str()).count();

        if count == 0 {
            // Try fuzzy match suggestions
            let suggestion = find_similar_string(&content, &p.old_string);
            return Err(ToolError::PatchStringNotFound {
                old_string: p.old_string,
                suggestion,
            });
        }

        if count > 1 && !p.replace_all {
            return Err(ToolError::PatchAmbiguous { count });
        }

        let new_content = if p.replace_all {
            content.replace(&p.old_string, &p.new_string)
        } else {
            content.replacen(&p.old_string, &p.new_string, 1)
        };

        tokio::fs::write(&path, &new_content).await.map_err(ToolError::Io)?;

        let replacements = if p.replace_all { count } else { 1 };
        Ok(ToolResult::text(format!(
            "Patched {} ({} replacement{})",
            path.display(),
            replacements,
            if replacements == 1 { "" } else { "s" }
        )))
    }
}
```

Fuzzy matching via `strsim` crate:
```rust
fn find_similar_string(haystack: &str, needle: &str) -> Option<String> {
    haystack
        .lines()
        .filter_map(|line| {
            let score = strsim::normalized_levenshtein(needle, line);
            if score > 0.6 { Some((score, line.to_string())) } else { None }
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap())
        .map(|(_, line)| line)
}
```

---

## 6. SearchFiles Tool

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchFilesParams {
    /// Regex pattern (content mode) or glob (files mode)
    pub pattern: String,
    #[serde(default = "default_search_target")]
    pub target: SearchTarget,
    pub path: Option<String>,
    pub file_glob: Option<String>,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchTarget { Content, Files }

impl Tool for SearchFilesTool {
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let p: SearchFilesParams = serde_json::from_value(args)?;
        let search_path = p.path
            .map(|s| ctx.fs.validate(Path::new(&s)))
            .transpose()?
            .unwrap_or_else(|| ctx.workdir.clone());

        let output = match p.target {
            SearchTarget::Content => {
                let mut cmd = tokio::process::Command::new("rg");
                cmd.arg("--json")
                   .arg("--max-count").arg("1")
                   .arg("-l")
                   .arg(&p.pattern)
                   .arg(&search_path);

                if let Some(glob) = &p.file_glob {
                    cmd.arg("--glob").arg(glob);
                }

                let out = cmd.output().await.map_err(|_| ToolError::RipgrepNotFound)?;
                parse_rg_json_output(&out.stdout, p.limit, p.offset)?
            }
            SearchTarget::Files => {
                let mut cmd = tokio::process::Command::new("rg");
                cmd.arg("--files")
                   .arg("--glob").arg(&p.pattern)
                   .arg(&search_path);
                let out = cmd.output().await.map_err(|_| ToolError::RipgrepNotFound)?;
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .skip(p.offset)
                    .take(p.limit)
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        };

        Ok(ToolResult::text(output))
    }
}
```
---

## Related Documents

### Depends On
- [Tool System Architecture](../02_Architecture/16_Tool_System_Architecture.md)

### See Also
- [Terminal Tool](30a_Terminal_Tool.md)
- [Skill Store](../07_Memory_System/57_Skill_Store.md)
- [Profile Isolation](40_Profile_Isolation.md)

