# 77 — TUI Landscape Overview: Frameworks & AI Agent Interfaces

> **Context:** Talon needs a terminal user interface. Hermes Agent uses Ink (React/TypeScript).
> Talon is Rust — what are the options, and what does the industry use?

---

## TUI Framework Comparison

### 1. Ratatui (Rust) ★ RECOMMENDED

- **Repo:** [ratatui/ratatui](https://github.com/ratatui/ratatui) · ~20.2k ⭐
- **Architecture:** Immediate mode — you redraw the entire UI every frame. No retained widget tree.
- **Backend:** Crossterm (cross-platform, async-friendly)
- **Event loop:** You own it. Integrates with tokio via crossterm's async event stream.
- **Widgets:** Paragraph, List, Table, Tabs, Block, Gauge, Sparkline, Chart, Canvas, Scrollbar
- **Ecosystem:** 3,700+ crates. Key ones:
  - `tui-textarea` — multi-line text input with editing
  - `tui-input` — single-line input
  - `ratatui-image` — terminal image rendering (Sixel, Kitty, iTerm2)
  - `tui-scrollview` — scrollable content areas
  - `tui-markdown` — markdown rendering widget

**Why for Talon:**
- Rust-native, zero FFI
- Full control over streaming LLM output rendering
- Split panes via Layout system
- AI agent TUIs already built with it (tenere, oatmeal)
- Immediate mode = perfect for streaming text that changes every frame

**Trade-off:** More boilerplate than retained-mode frameworks. You manage all state yourself.

### 2. Ink (TypeScript) — What Hermes Uses

- **Repo:** [vadimdemedes/ink](https://github.com/vadimdemedes/ink) · ~27k ⭐
- **Architecture:** Retained mode — React component tree with Yoga flexbox layout
- **Who uses it:** Claude Code, Gemini CLI, Qwen Code, OpenAI Codex CLI, Hermes Agent
- **The dominant AI agent TUI framework in 2025–2026**

**Why NOT for Talon:** Node.js dependency. Talon is Rust, needs native performance.

### 3. Textual (Python)

- **Repo:** [Textualize/textual](https://github.com/Textualize/textual) · ~27k ⭐
- **Architecture:** Retained mode with CSS-based styling
- **Key features:** CSS stylesheets for TUI, async-first, rich widgets, hot-reload CSS

**Why NOT for Talon:** Python only.

### 4. Bubbletea (Go)

- **Repo:** [charmbracelet/bubbletea](https://github.com/charmbracelet/bubbletea) · ~30k ⭐
- **Architecture:** Elm/MVU (Model-View-Update) — functional pattern
- **Ecosystem:** Lip Gloss (styling), Bubbles (components), Glamour (markdown rendering)
- **Who uses it:** OpenCode

**Why study it:** Best reference for TUI architecture patterns. The Elm/MVU model is clean and could be adopted on top of Ratatui.

### 5. Cursive (Rust)

- **Repo:** [gyscos/cursive](https://github.com/gyscos/cursive) · ~4.2k ⭐
- **Architecture:** Retained mode — widget tree with callbacks
- **Vs Ratatui:** Higher-level but less flexible. Smaller ecosystem, less active.

**Why NOT for Talon:** Less suitable for highly custom rendering like streaming markdown.

---

## Low-Level Terminal Backends (Rust)

| | Crossterm ✅ | Termion | Termwiz |
|---|---|---|---|
| Stars | ~3.3k | ~2.1k | ~400 |
| Platform | Windows + Unix | Unix only | Windows + Unix |
| Async | tokio EventStream | Thread-based | No |
| Maintained by | Community (active) | Redox OS | Wez Furlong |

**Decision: Crossterm.** Cross-platform, async-friendly, Ratatui's default backend.

---

## How AI CLI Tools Build Their TUIs

| Tool | Language | Framework | Rendering Pattern |
|---|---|---|---|
| **Claude Code** | TypeScript | Ink (custom fork) | React components → ANSI |
| **Gemini CLI** | TypeScript | Ink | React components → ANSI |
| **Codex CLI** | TypeScript→Rust | Ink (original) | React → ANSI |
| **OpenCode** | Go | Bubbletea + Glamour | Elm/MVU + markdown themes |
| **Aider** | Python | Rich + prompt_toolkit | Streaming `Live` display |
| **GitHub Copilot CLI** | TypeScript | Ink | React components |
| **Amazon Q CLI** | Rust | Custom + crossterm | Direct terminal manipulation |
| **Warp** | Rust | Custom GPU renderer | Metal/wgpu (terminal emulator) |

**Key insight:** The TypeScript/Ink world dominates AI CLIs today. Talon has the opportunity to be the **first major Rust-native AI agent TUI** built properly on Ratatui.

---

## Recommendation for Talon

**Ratatui + Crossterm**, adopting Bubbletea's MVU architecture pattern:

```
┌──────────────────────────────────────────────────┐
│  Talon TUI Architecture                          │
│                                                   │
│  ┌─────────────────────────────────────────────┐ │
│  │              MVU Event Loop                  │ │
│  │  Init() → Update(Msg) → View(Frame) → ...  │ │
│  └─────────────────────┬───────────────────────┘ │
│                        │                          │
│  ┌─────────┐  ┌────────▼────────┐  ┌───────────┐ │
│  │ Async   │  │   Ratatui       │  │ Crossterm │ │
│  │ Channels│──│   Rendering     │──│ Backend   │ │
│  │ (tokio) │  │   (immediate)   │  │           │ │
│  └─────────┘  └─────────────────┘  └───────────┘ │
│                                                   │
│  Components:                                      │
│  ├── ChatView (streaming markdown + code blocks) │
│  ├── InputBar (multi-line, history, autocomplete)│
│  ├── ToolPanel (execution status, output)        │
│  ├── StatusBar (model, tokens, session)          │
│  └── SplitPane (adaptive layout by term width)   │
└──────────────────────────────────────────────────┘
```

OpenCode (Go/Bubbletea) is the **best reference implementation** to study for feature parity — it has the closest UX to what Talon needs.
