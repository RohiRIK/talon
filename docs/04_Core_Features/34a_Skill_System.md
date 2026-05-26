# Skill System

> **Status:** ✅ Complete
> **Category:** Core Features

---

## 1. Overview

Skills are markdown documents that give Talon procedural memory.
They answer the question: "How should Talon approach task X?"

Unlike plugins (compiled Rust code), skills are pure text — no code.
The LLM reads them and follows the instructions.

---

## 2. Skill Lifecycle

```
1. User (or Talon) authors SKILL.md
2. File placed in ~/.talon/profiles/<name>/skills/
3. On startup: SkillRegistry scans and parses frontmatter
4. System prompt lists skill names + descriptions (<available_skills>)
5. LLM calls skill_view(name) to load full content
6. Skill body injected into context under <loaded_skills>
7. LLM follows the skill's instructions
```

Talon also creates skills autonomously after solving a novel problem:
"This was a 5+ step non-obvious workflow. Should I save it as a skill?"

---

## 3. Mandatory Skill Loading

Some skills are so critical that Talon MUST load them before answering
certain queries — the system prompt instructs this:

```
## Skills (mandatory)
Before replying, scan the skills below. If a skill matches or is even
partially relevant to your task, you MUST load it with skill_view(name)
and follow its instructions.
```

This prevents Talon from hallucinating a workflow when a correct one
is already documented.

---

## 4. When to Create a Skill

Trigger conditions:
- Task required 5+ tool calls
- Non-obvious approach was discovered
- User corrected Talon's default approach
- User explicitly says "remember how to do this"
- Error was encountered and overcome (pitfall to document)

Anti-patterns (don't create skills for):
- One-off tasks that won't recur
- Simple commands (`git status`, `cargo build`)
- Tasks covered by a more general skill

---

## 5. Skill Quality Checklist

A good skill has:
- [ ] Clear trigger conditions (when to load it)
- [ ] Prerequisites (what must be set up first)
- [ ] Numbered steps with exact commands
- [ ] Pitfalls section (what can go wrong)
- [ ] Verification step (how to confirm success)
- [ ] YAML frontmatter with `name`, `description`, `triggers`

---

## 6. Built-in Talon Skills

Talon ships with a default skill library:

| Skill | Category | Purpose |
|-------|----------|---------|
| `dev-workflow` | software-development | Entry point for any dev task |
| `spec` | software-development | Spec a feature before building |
| `plan` | software-development | Write implementation plan |
| `build-loop` | software-development | Execute an approved plan |
| `verify` | software-development | Pre-commit quality gate |
| `capture` | software-development | End-of-session summary |
| `github-pr-workflow` | github | Full PR lifecycle |
| `systematic-debugging` | software-development | 4-phase root cause debugging |
| `memory-routing` | devops | Where to store what kind of info |

These are bundled as static strings in `talon-core` and written to the
skills directory on first run.

---

## 7. Skill Tools Reference

| Tool | Parameters | Description |
|------|-----------|-------------|
| `skill_view` | `name`, `file_path?` | Load a skill's content |
| `skills_list` | `category?` | List available skills |
| `skill_manage` | `action`, `name`, `content?`, `old_string?`, `new_string?` | CRUD operations |

`skill_manage` actions: `create`, `patch`, `edit`, `delete`, `write_file`, `remove_file`

---

## 8. Pinned Skills

Pinned skills are protected from deletion by the LLM.
Only the user can unpin them:

```bash
talon curator unpin github-pr-workflow
```

The `delete` action on a pinned skill returns:
```
Skill 'github-pr-workflow' is pinned and cannot be deleted.
Use `talon curator unpin github-pr-workflow` to unpin first.
```
---

## Related Documents

### Depends On
- [Plugin & Skill Architecture](../02_Architecture/17_Plugin_And_Skill_Architecture.md)

### See Also
- [Skill Store](../07_Memory_System/57_Skill_Store.md)
- [Skill File Management](../07_Memory_System/57a_Skill_File_Management.md)
- [Self-Evolution Loop](39_Self_Evolution_Loop.md)

