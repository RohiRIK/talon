# Skill File Management

> **Status:** ✅ Complete
> **Category:** Memory System

---

## 1. What is a Skill?

A skill is a markdown document that gives Talon procedural memory —
"how to do X" for recurring tasks. Skills are:
- **Human-authored** (user writes them) or **agent-authored** (Talon creates them after solving a novel problem)
- **Loaded on demand** (injected into context when relevant)
- **Versioned** (plain files, git-trackable)
- **Pinned** (protected against agent deletion)

---

## 2. SKILL.md Format

```markdown
---
name: github-pr-workflow
description: GitHub PR lifecycle — branch, commit, open, CI, merge
triggers:
  - When asked to open a PR
  - "create pull request"
  - Any commit + push workflow
pinned: false
---

## When to Use
Load this skill whenever the user asks to open a PR, commit code, or
interact with GitHub in a PR lifecycle context.

## Prerequisites
- `gh` CLI installed and authenticated (`gh auth status`)
- Git configured with upstream remote

## Steps
1. Create branch: `git checkout -b feat/<name>`
2. Stage changes: `git add -p` (interactive) or `git add .`
3. Commit: `git commit -m "feat: <description>"` (conventional commits)
4. Push: `git push -u origin HEAD`
5. Open PR: `gh pr create --fill`
6. Monitor CI: `gh pr checks --watch`
7. Merge when green: `gh pr merge --squash`

## Pitfalls
- Never force-push to main
- Always check `gh pr checks` before merging
- If CI fails, check logs with `gh run view --log-failed`
```

---

## 3. Skill Discovery

Skills are discovered from the profile's skills directory:

```rust
pub struct SkillRegistry {
    skills: HashMap<String, SkillMeta>,
    skills_dir: PathBuf,
}

pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub triggers: Vec<String>,
    pub pinned: bool,
    pub path: PathBuf,
    pub category: Option<String>,
}

impl SkillRegistry {
    pub async fn scan(&mut self) -> Result<(), SkillError> {
        let pattern = self.skills_dir.join("**/*.md");
        for entry in glob::glob(pattern.to_str().unwrap())?.flatten() {
            if let Ok(meta) = self.parse_frontmatter(&entry).await {
                self.skills.insert(meta.name.clone(), meta);
            }
        }
        Ok(())
    }

    async fn parse_frontmatter(&self, path: &Path) -> Result<SkillMeta, SkillError> {
        let content = tokio::fs::read_to_string(path).await?;
        // Extract YAML frontmatter between --- delimiters
        let fm = extract_frontmatter(&content)
            .ok_or(SkillError::MissingFrontmatter(path.display().to_string()))?;
        let meta: SkillFrontmatter = serde_yaml::from_str(fm)?;
        Ok(SkillMeta {
            name: meta.name,
            description: meta.description,
            triggers: meta.triggers.unwrap_or_default(),
            pinned: meta.pinned.unwrap_or(false),
            path: path.to_path_buf(),
            category: path.parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .filter(|&s| s != "skills")
                .map(str::to_string),
        })
    }
}
```

---

## 4. Skill Loading

When the LLM or a system component requests a skill:

```rust
pub async fn load_skill(&self, name: &str) -> Result<LoadedSkill, SkillError> {
    let meta = self.skills.get(name)
        .ok_or_else(|| SkillError::NotFound(name.to_string()))?;

    let content = tokio::fs::read_to_string(&meta.path).await?;

    // Strip frontmatter; return only the body
    let body = strip_frontmatter(&content);

    Ok(LoadedSkill {
        name: meta.name.clone(),
        description: meta.description.clone(),
        content: body.to_string(),
    })
}
```

Skills are injected into the system prompt at Layer 5:

```
<loaded_skills>
## github-pr-workflow
When to Use: load when user asks to open a PR...
[full body]
</loaded_skills>
```

---

## 5. Skill CRUD Tools

The LLM can manage skills via tools:

```rust
// Tool: skill_manage
pub async fn skill_manage_handler(params: SkillManageParams) -> ToolResult {
    match params.action {
        SkillAction::Create { name, content, category } => {
            let path = build_skill_path(&registry.skills_dir, &name, category.as_deref());
            // Guard: never overwrite pinned skill
            if let Some(existing) = registry.get(&name) {
                if existing.pinned {
                    return ToolResult::error(format!(
                        "Skill '{}' is pinned and cannot be overwritten. \
                         Unpin it first with `hermes curator unpin {}`", name, name
                    ));
                }
            }
            tokio::fs::write(&path, content).await?;
            registry.reload_skill(&path).await?;
            ToolResult::success(format!("Skill '{}' created at {:?}", name, path))
        }
        SkillAction::Patch { name, old_string, new_string } => {
            let meta = registry.require(&name)?;
            let content = tokio::fs::read_to_string(&meta.path).await?;
            if !content.contains(&old_string) {
                return ToolResult::error("old_string not found in skill file");
            }
            let updated = content.replacen(&old_string, &new_string, 1);
            tokio::fs::write(&meta.path, updated).await?;
            ToolResult::success(format!("Skill '{}' patched", name))
        }
        SkillAction::Delete { name, absorbed_into } => {
            let meta = registry.require(&name)?;
            if meta.pinned {
                return ToolResult::error(format!(
                    "Skill '{}' is pinned. Use `hermes curator unpin {}`", name, name
                ));
            }
            tokio::fs::remove_file(&meta.path).await?;
            registry.remove(&name);
            ToolResult::success(format!("Skill '{}' deleted", name))
        }
    }
}
```

---

## 6. Skill Index (skills_list tool)

```rust
pub fn skills_list(&self, category: Option<&str>) -> Vec<SkillSummary> {
    self.skills.values()
        .filter(|s| {
            category.map(|c| s.category.as_deref() == Some(c)).unwrap_or(true)
        })
        .map(|s| SkillSummary {
            name: s.name.clone(),
            description: s.description.clone(),
            category: s.category.clone(),
            pinned: s.pinned,
        })
        .collect()
}
```

Output example:
```
github-pr-workflow    GitHub PR lifecycle (branch, commit, open, CI, merge)
dev-workflow          Entry point for any dev task
spec                  Spec a feature before building
requesting-code-review  Pre-commit security scan + quality gates
```
---

## Related Documents

### Depends On
- [Skill Store](57_Skill_Store.md)

### See Also
- [File System Tool](../04_Core_Features/31_File_System_Tool.md)
- [Profile Isolation](../04_Core_Features/40_Profile_Isolation.md)

