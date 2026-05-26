# Gap Analysis — What's Missing or Deferred

> **Source:** graphify query results from `Missing Coverage` god nodes (12 edges each)
> + Community 9 (Hermes audit) + Community 10 (OpenClaw audit) + Community 34 (OpenClaw unique features)
> + Community 42 (OpenClaw unverified claims)

---

## How to Read This

Each gap is classified as:

- 🔴 **Must Fix** — blocking for MVP correctness
- 🟡 **Backlog** — real feature, deferred to post-v1
- 🟢 **Intentional Drop** — explicitly decided not to build, with rationale
- ⚪ **Research Needed** — feature may exist in source but our docs are unclear

---

## Gap Category 1 — Platform & Gateway Coverage

### 1.1 — WeChat / QQ Gateways
- **Status:** 🟢 Intentional Drop
- **Source finding:** Community 9 audit — `docs/01_Analysis/02_OpenClaw_Deep_Dive.md` says DROP, but WeChat/QQ gateways actually exist in OpenClaw source
- **Rationale:** WeChat requires mainland China business registration. QQ API unstable. Out of scope for v1.
- **Docs affected:** Doc 18 (Gateway Architecture) — add explicit note in "Platform Support Matrix"
- **If ever needed:** Implement `Gateway` trait for `WeChatGateway`. No other changes needed (trait is the extension point).

### 1.2 — iMessage / BlueBubbles Gateway
- **Status:** 🟡 Backlog
- **Source finding:** iMessage via BlueBubbles exists in Hermes source but docs say DROP
- **Rationale:** Requires macOS host + BlueBubbles server — too environment-specific for v1
- **Docs affected:** Doc 18

### 1.3 — Google Gemini Native Adapter
- **Status:** 🟡 Backlog (high value)
- **Source finding:** Community 10 — Hermes has a native Gemini adapter
- **Current state:** `OPENAI_BASE_URL=https://generativelanguage.googleapis.com/v1beta/openai/` works as OpenAI-compat
- **Rationale for deferral:** OpenAI-compat works for most models. Native adapter enables: Gemini-specific features (grounding, code execution), proper thinking model support, cost optimization via direct API
- **Docs affected:** Doc 41 (LLM Abstraction), Doc 47 (Ollama/Other Providers)
- **Implementation when ready:** New crate `talon-llm-gemini` implementing `LlmProvider`

### 1.4 — Microsoft Foundry / Azure OpenAI
- **Status:** 🟡 Backlog
- **Source finding:** Community 10 — mentioned as present in Hermes
- **Current state:** Azure works via OpenAI-compat with `OPENAI_BASE_URL=https://<resource>.openai.azure.com/`
- **Docs affected:** Doc 42 (OpenAI-Compatible Client) — add Azure config example

### 1.5 — SMS / WhatsApp / Signal Gateways
- **Status:** 🟡 Backlog
- **Source finding:** Community 42 (OpenClaw unverified) — WhatsApp positioning mentioned
- **Rationale:** Twilio/Signal require paid accounts and phone numbers. Deferred to post-v1.

---

## Gap Category 2 — TUI Architecture

### 2.1 — Hermes Uses React/Ink, Not Python Rich
- **Status:** 🟢 Intentional difference (correctly documented)
- **Source finding:** Community 9 — "TUI is React/Ink (TypeScript), NOT Rich-based"
- **Talon's approach:** `ratatui` (Rust-native) — documented in Doc 76 (TUI Implementation)
- **This is correct:** We are building a Rust-native TUI, not replicating Hermes's React/Ink approach
- **No action needed:** Doc 76 correctly documents ratatui. Add a note explaining why we differ.

### 2.2 — Web Dashboard for Local Agent Management
- **Status:** 🟡 Backlog
- **Source finding:** Community 10 — web dashboard exists in OpenClaw
- **Talon's approach:** v1 targets CLI + Telegram. Web dashboard is a v2 feature.
- **Docs affected:** Doc 18 (Gateway) — add `HttpGateway` as a stub/future gateway type

---

## Gap Category 3 — Skill System

### 3.1 — Skill Slash Commands Inject as User Message (Not System Prompt)
- **Status:** ⚪ Research Needed → resolved
- **Source finding:** Community 9 — "Skill slash commands inject as user message, not system prompt"
- **Implication for Doc 38 (Skill Store):** The skill loading mechanism must prepend skill content to the user turn, NOT add it to the system prompt
- **Current docs:** Doc 38 documents `SKILL.md` format and `SkillStore` but may not be explicit about injection point
- **Fix needed:** Add explicit section in Doc 38 clarifying: "Skill content is injected as a user-turn prefix, not as a system prompt addition."

### 3.2 — ClawHub Skills Marketplace
- **Status:** 🟡 Backlog
- **Source finding:** Community 42 — ClawHub mentioned as public skills marketplace
- **Talon's v1 approach:** Local skills only (`~/.talon/skills/`)
- **Future path:** Implement `talon skill install <username>/<skill>` pulling from GitHub repos (like `pip install` for skills)

---

## Gap Category 4 — Self-Evolution

### 4.1 — Self-Evolution Tech Stack Was Fabricated
- **Status:** 🔴 Already Fixed (Pass 2)
- **Source finding:** Community 3 — "Self-evolution tech stack is entirely fabricated (Agent 3)"
- **What we had wrong:** Docs implied a 4-phase fine-tuning pipeline producing model weights
- **Reality:** GEPA + DSPy prompt evolution. No GPU. No fine-tuning. Evolves prompts and skills, not weights.
- **Fix status:** Doc 39 (Self-Evolution Loop) was rewritten in audit pass 2. Verify the current version is correct.
- **Key correction in Doc 39:** Section "2. Python + DSPy Implementation" correctly describes DSPy + GEPA. Section "5. Talon (Rust) Implementation" describes the Rust evaluation harness.

### 4.2 — Trajectory Storage is Talon-Specific, Not From Hermes
- **Status:** 🟢 Informational
- **Source finding:** Community 80 — "SQLite trajectory storage is Ernest-specific, not from Hermes"
- **Implication:** Doc 33 (Batch Trajectory Generation) describes a Talon innovation, not a migration of existing code
- **No correction needed** — just useful to know this is new code, not a port

---

## Gap Category 5 — OpenClaw Unique Features

### 5.1 — OpenClaw "SOUL.md" / Persona System
- **Status:** 🟡 Backlog
- **Source finding:** Community 34 — OpenClaw has a SOUL.md persona configuration file
- **Talon equivalent:** Not implemented. Could be a named config section `[agent.persona]` in config.toml
- **Docs affected:** Doc 64 (Config System) — add future `[agent.persona]` config block

### 5.2 — "Onboard" Guided Setup System
- **Status:** 🟡 Backlog
- **Source finding:** Community 34 — OpenClaw has guided onboarding flow
- **Talon approach:** `talon setup` wizard command. Not in v1 scope.

### 5.3 — External AI Coding Agent Integration (e.g., "Pi")
- **Status:** 🟢 Covered by ACP Protocol
- **Source finding:** Community 34 — OpenClaw integrates with external AI coding agents
- **Talon equivalent:** ACP protocol (Doc 88 area + Doc 19 Subagent Architecture). This IS implemented.

### 5.4 — OpenClaw.NET Variant
- **Status:** 🟢 Intentional Drop
- **Source finding:** Community 34 — .NET version of OpenClaw exists
- **Rationale:** We are building Talon as a Rust-native agent. .NET variant is irrelevant.

### 5.5 — LangChain as Partial Dependency in OpenClaw
- **Status:** 🟢 Intentional Drop
- **Source finding:** Community 34 — "LangChain listed as dependency with 'partial' use"
- **Talon approach:** No LangChain. Direct API clients only. This is a core migration win.

---

## Gap Category 6 — Documentation Correctness Issues

### 6.1 — Skills System Path Uses `~/.ernest/` in Some Docs
- **Status:** 🔴 Must Verify
- **Source finding:** Community 10 — "Skills system path in Ernest's Skill System doc uses `~/.ernest/` not `~/.hermes/`"
- **Post-rename:** After the Ernest → Talon rename, all paths should use `~/.talon/`
- **Check:** `grep -r '~/.ernest' docs/` should return 0 results
- **Check:** `grep -r '~/.hermes' docs/` should also return 0 results (we're Talon, not Hermes)
- **Fix command:** `sed -i 's|~/.ernest|~/.talon|g' docs/**/*.md`

### 6.2 — File Path Errors in Cross-References
- **Status:** 🔴 Must Verify
- **Source finding:** Community 34 — "File path error — `09_Keep_Edit_Drop_Analysis.md` does not exist" and "`21_Migration_Phases_Overview.md` does not exist"
- **Correct paths:** `08_Feature_Mapping_Keep_Edit_Drop.md` and `21_Migration_Roadmap.md`
- **Check:** Run `grep -r 'Keep_Edit_Drop_Analysis' docs/` and `grep -r 'Migration_Phases_Overview' docs/`

### 6.3 — Startup Time / Memory RSS Internal Inconsistency
- **Status:** 🟡 Low priority
- **Source finding:** Community 42 — startup time and memory RSS numbers are inconsistent between docs
- **These are benchmark estimates, not guaranteed specs** — add `*estimated*` qualifier to all performance numbers

### 6.4 — `~22,000 lines of Python` Is an Undercount
- **Status:** 🟢 Informational
- **Source finding:** Community 9 — actual line count is higher
- **Implication:** The migration effort is larger than initially estimated in Doc 07 (Python Pain Points)
- **No correction needed** — the exact number doesn't matter for architecture decisions

---

## Gap Category 7 — Voice / Audio

### 7.1 — Voice Integration Not Covered Enough
- **Status:** 🟡 Backlog
- **Source finding:** Community 17 covers voice (STT/TTS, Whisper, Telegram voice)
- **Current doc coverage:** Doc 17 (Voice I/O) exists but is in the plugin category
- **What's missing:** Integration with Telegram voice messages is a high-value user feature (push-to-talk to AI agent)
- **When to build:** After Telegram gateway (Doc 45) is stable

---

## Summary Table

| Gap | Category | Status | Priority |
|-----|----------|--------|----------|
| WeChat/QQ gateway | Platform | 🟢 Drop | — |
| iMessage/BlueBubbles | Platform | 🟡 Backlog | Post-v1 |
| Gemini native adapter | LLM | 🟡 Backlog | High |
| Azure OpenAI | LLM | 🟡 Backlog | Medium |
| Skill injection point (user message) | Skills | 🔴 Fix Doc 38 | Before Phase 6 |
| ClawHub marketplace | Skills | 🟡 Backlog | Post-v1 |
| Self-evolution corrected | Evolution | 🟢 Fixed | Done |
| SOUL.md persona system | Config | 🟡 Backlog | Post-v1 |
| `~/.ernest/` path remnants | Docs | 🔴 Verify | Immediate |
| File path cross-ref errors | Docs | 🔴 Verify | Immediate |
| Startup time inconsistency | Docs | 🟡 Low | Before release |
| Voice/STT integration | Feature | 🟡 Backlog | Post-v1 |

---

*Based on graphify community analysis — Communities 3, 9, 10, 34, 42, 80*
