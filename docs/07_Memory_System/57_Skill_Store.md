# Skill Store & Hot-Reload

> **Status:** ✅ Complete
> **Category:** Memory System

---

## 1. Overview

Skills are markdown files with YAML frontmatter living in the profile directory.
The SkillStore scans them at startup, caches parsed metadata, and watches
for filesystem changes via `notify` to hot-reload without restart.

```
~/.talon/skills/
├── github-pr-workflow/
│   ├── SKILL.md
│   ├── references/
│   │   └── api.md
│   └── templates/
│       └── pr-body.md
├── homelab-docker-swarm/
│   └── SKILL.md
└── my-custom-skill/
    └── SKILL.md
```

---

## 2. SKILL.md Format

```markdown
---
name: github-pr-workflow
description: GitHub PR lifecycle: branch, commit, open, CI, merge.
category: github
pinned: false
tags: [git, github, pr, workflow]
---

# GitHub PR Workflow

...skill content here...
```

---

## 3. SkillStore Implementation

```rust
pub struct SkillStore {
    skills_dir: PathBuf,
    cache: Arc<RwLock<SkillCache>>,
    watcher: Option<notify::RecommendedWatcher>,
}

#[derive(Default)]
struct SkillCache {
    /// name → Skill (metadata only, content loaded on demand)
    index: HashMap<String, Skill>,
    /// Last full scan timestamp
    scanned_at: Option<Instant>,
}

impl SkillStore {
    pub async fn new(skills_dir: PathBuf) -> Result<Arc<Self>, SkillError> {
        let store = Arc::new(Self {
            skills_dir: skills_dir.clone(),
            cache: Arc::new(RwLock::new(SkillCache::default())),
            watcher: None,
        });

        // Initial scan
        store.scan_all().await?;

        // Start filesystem watcher
        store.clone().start_watcher(skills_dir).await?;

        Ok(store)
    }

    async fn scan_all(&self) -> Result<usize, SkillError> {
        let dir = self.skills_dir.clone();
        let skills = tokio::task::spawn_blocking(move || {
            scan_skills_dir(&dir)
        }).await??;

        let count = skills.len();
        let mut cache = self.cache.write().await;
        cache.index = skills.into_iter().map(|s| (s.name.clone(), s)).collect();
        cache.scanned_at = Some(Instant::now());

        tracing::info!(count, "Skills loaded");
        Ok(count)
    }
}

fn scan_skills_dir(dir: &Path) -> Result<Vec<Skill>, SkillError> {
    let mut skills = vec![];

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        let skill_md_path = if path.is_dir() {
            path.join("SKILL.md")
        } else if path.extension().map(|e| e == "md").unwrap_or(false) {
            path.clone()
        } else {
            continue;
        };

        if !skill_md_path.exists() { continue; }

        match parse_skill_file(&skill_md_path) {
            Ok(skill) => skills.push(skill),
            Err(e) => {
                tracing::warn!(path = %skill_md_path.display(), error = %e, "Skipping invalid skill");
            }
        }
    }

    Ok(skills)
}
```

---

## 4. Frontmatter Parsing

```rust
pub fn parse_skill_file(path: &Path) -> Result<Skill, SkillError> {
    let content = std::fs::read_to_string(path)
        .map_err(SkillError::Io)?;

    let (frontmatter, _body) = split_frontmatter(&content)?;

    let fm: SkillFrontmatter = serde_yaml::from_str(&frontmatter)
        .map_err(|e| SkillError::InvalidFrontmatter {
            path: path.to_path_buf(),
            error: e.to_string(),
        })?;

    let updated_at = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(|_| Utc::now());

    Ok(Skill {
        name: fm.name,
        description: fm.description,
        category: fm.category,
        pinned: fm.pinned.unwrap_or(false),
        tags: fm.tags.unwrap_or_default(),
        path: path.to_path_buf(),
        updated_at,
        content_cache: None,
    })
}

fn split_frontmatter(content: &str) -> Result<(String, String), SkillError> {
    let content = content.trim_start();

    if !content.starts_with("---") {
        // No frontmatter — return empty frontmatter, full content as body
        return Ok((String::new(), content.to_string()));
    }

    let after_open = &content[3..];
    let close_pos = after_open.find("\n---")
        .ok_or(SkillError::UnclosedFrontmatter)?;

    let frontmatter = after_open[..close_pos].trim().to_string();
    let body = after_open[close_pos + 4..].trim_start().to_string();

    Ok((frontmatter, body))
}
```

---

## 5. Hot-Reload via notify

```rust
impl SkillStore {
    async fn start_watcher(
        self: Arc<Self>,
        dir: PathBuf,
    ) -> Result<(), SkillError> {
        let (tx, mut rx) = mpsc::channel::<notify::Event>(32);

        let mut watcher = notify::recommended_watcher(move |res| {
            if let Ok(event) = res {
                let _ = tx.blocking_send(event);
            }
        })?;

        watcher.watch(&dir, notify::RecursiveMode::Recursive)?;

        tokio::spawn(async move {
            let _watcher = watcher;  // keep alive

            while let Some(event) = rx.recv().await {
                // Only care about create/modify/remove of .md files
                let relevant = event.paths.iter().any(|p| {
                    p.extension().map(|e| e == "md").unwrap_or(false)
                });

                if relevant {
                    match event.kind {
                        notify::EventKind::Create(_)
                        | notify::EventKind::Modify(_) => {
                            for path in &event.paths {
                                if let Ok(skill) = parse_skill_file(path) {
                                    tracing::info!(name = skill.name, "Skill reloaded");
                                    let mut cache = self.cache.write().await;
                                    cache.index.insert(skill.name.clone(), skill);
                                }
                            }
                        }
                        notify::EventKind::Remove(_) => {
                            let mut cache = self.cache.write().await;
                            cache.index.retain(|_, s| s.path.exists());
                        }
                        _ => {}
                    }
                }
            }
        });

        Ok(())
    }
}
```

---

## 6. skill_view / skills_list Tools

```rust
// skills_list tool
pub async fn list_skills(&self, category: Option<&str>) -> Vec<SkillSummary> {
    let cache = self.cache.read().await;
    let mut skills: Vec<_> = cache.index.values()
        .filter(|s| category.map(|c| s.category.as_deref() == Some(c)).unwrap_or(true))
        .map(|s| SkillSummary {
            name: s.name.clone(),
            description: s.description.clone(),
            category: s.category.clone(),
            pinned: s.pinned,
        })
        .collect();

    // Pinned first, then alphabetical
    skills.sort_by(|a, b| {
        b.pinned.cmp(&a.pinned)
            .then(a.name.cmp(&b.name))
    });
    skills
}

// skill_view tool — returns full SKILL.md content
pub async fn view_skill(&self, name: &str, file_path: Option<&str>) -> Result<String, SkillError> {
    let cache = self.cache.read().await;
    let skill = cache.index.get(name)
        .ok_or_else(|| SkillError::NotFound(name.to_string()))?;

    let base_dir = skill.path.parent().unwrap_or(&skill.path);

    let target = match file_path {
        None => skill.path.clone(),
        Some(fp) => {
            let p = base_dir.join(fp);
            // Security: must stay inside skill directory
            let canonical = p.canonicalize()
                .map_err(|_| SkillError::LinkedFileNotFound(fp.to_string()))?;
            if !canonical.starts_with(base_dir) {
                return Err(SkillError::PathEscape);
            }
            canonical
        }
    };

    tokio::fs::read_to_string(&target).await.map_err(SkillError::Io)
}
```

---

## 7. skill_manage Tool

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SkillManageParams {
    pub action: SkillManageAction,
    pub name: String,
    pub content: Option<String>,
    pub old_string: Option<String>,
    pub new_string: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillManageAction {
    Create, Edit, Patch, Delete,
}

// Delete respects pinned flag
pub async fn delete_skill(&self, name: &str) -> Result<(), SkillError> {
    let cache = self.cache.read().await;
    let skill = cache.index.get(name)
        .ok_or_else(|| SkillError::NotFound(name.to_string()))?;

    if skill.pinned {
        return Err(SkillError::Pinned {
            name: name.to_string(),
            hint: format!(
                "Run `talon curator unpin {}` first", name
            ),
        });
    }

    let path = skill.path.clone();
    drop(cache);

    tokio::fs::remove_file(&path).await.map_err(SkillError::Io)?;

    // notify watcher will handle cache removal
    Ok(())
}
```
---

## Related Documents

### Depends On
- [Plugin & Skill Architecture](../02_Architecture/17_Plugin_And_Skill_Architecture.md)
- [SQLite & FTS5 in Rust](55_SQLite_FTS5_In_Rust.md)

### See Also
- [Self-Evolution Loop](../04_Core_Features/39_Self_Evolution_Loop.md)
- [FTS5 Search Deep Dive](58_FTS5_Search_Deep_Dive.md)
- [Skill File Management](57a_Skill_File_Management.md)

