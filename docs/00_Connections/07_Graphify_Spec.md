# Graphify — Complete Spec for the Talon Project

> **Purpose:** A full technical reference for how graphify works, every feature it offers, and exactly how to use it on the Talon codebase as it evolves from docs → Rust source. This is your graphify "bible" — paste it to any AI and it will know what to do.

---

## Identity

| Field | Value |
|---|---|
| PyPI package | `graphifyy` (double-y — name disambiguation) |
| CLI command | `graphify` |
| Source | [github.com/safishamsi/graphify](https://github.com/safishamsi/graphify) (branch `v8`) |
| Stars | 53.8k ⭐ · YC S26 |
| License | MIT |
| Installed via | `uv tool install graphifyy` ✅ (already installed on this machine) |
| Current version | 0.8.18 |

---

## Core Concept

Graphify turns a codebase (or a docs folder) into a **queryable knowledge graph**:

```
Files on disk
    │
    ▼
detect()     ← walk directory, filter by extension
extract()    ← AST (local, free) + LLM semantic pass (optional, costs tokens)
build()      ← merge extraction dicts into NetworkX graph
cluster()    ← Leiden community detection (or Louvain fallback)
analyze()    ← god nodes, surprise edges, suggested questions
report()     ← GRAPH_REPORT.md
export()     ← graph.json, graph.html, wiki/, obsidian/, SVG, GraphML, Neo4j
```

All outputs land in `graphify-out/`. The graph is persistent — `update` only re-extracts changed files.

---

## Two Extraction Modes

| Mode | Trigger | Cost | What it extracts |
|---|---|---|---|
| **AST (local)** | `graphify update .` | Free, no API key | Functions, classes, imports, call edges — structural graph from 31 languages via tree-sitter |
| **Semantic (LLM)** | `graphify extract . --backend <llm>` | API tokens | Concepts, design rationale, cross-domain relationships, prose from markdown docs |

**Key rule:** For Rust source files, `graphify update .` gives you the full structural graph (free). For markdown docs, you need `graphify extract` with an LLM backend to get meaningful semantic edges.

**For Talon today:** The docs are markdown → semantic extraction needed. The Rust `crates/` (once written) → AST extraction, free.

---

## Full CLI Reference

### Build / Update

```bash
# Full build from scratch (AST only for code, LLM for docs)
graphify .

# Re-extract only changed code files (AST, no LLM, free)
graphify update .

# Force full re-extraction with LLM backend
graphify extract . --backend claude

# Specific flags
graphify . --mode deep          # more aggressive INFERRED edges
graphify . --cluster-only       # rerun clustering only
graphify . --no-viz             # skip HTML, produce JSON + report only
graphify . --wiki               # build Wikipedia-style wiki per community
graphify . --obsidian           # generate Obsidian vault with backlinks
graphify . --watch              # auto-sync as files change
```

### Query

```bash
# BFS traversal scoped to a question (default 2000 token budget)
graphify query "what connects the agent loop to the LLM provider?"

# With flags
graphify query "..." --dfs                 # depth-first instead of BFS
graphify query "..." --budget 3000         # larger token budget
graphify query "..." --graph path/to/graph.json

# Shortest path between two nodes
graphify path "AgentLoop" "LlmProvider"

# Full explanation of a node and its neighbors
graphify explain "ToolRegistry"
```

### Export

```bash
graphify . --svg                           # graph.svg
graphify . --graphml                       # graph.graphml (Gephi/yEd)
graphify . --neo4j                         # cypher.txt
graphify . --neo4j-push bolt://localhost   # push to live Neo4j
graphify . --mcp                           # start MCP stdio server
graphify export callflow-html              # Mermaid architecture HTML
```

### External Sources

```bash
graphify add https://arxiv.org/abs/...
graphify add https://x.com/...  --author "Name"
```

### AI Platform Integration

```bash
# Install graphify skill into your AI platform config
graphify install                # auto-detects platform
graphify hermes install         # ← already done ✅ (wrote to ~/AGENTS.md)
graphify claude install         # CLAUDE.md + PreToolUse hook
graphify codex install          # AGENTS.md
graphify opencode install       # AGENTS.md

# Git hooks (auto-rebuild AST on commit)
graphify hook install
graphify hook status
```

### PR Integration

```bash
graphify prs                    # PR dashboard
graphify prs --triage           # AI triage
graphify prs --conflicts        # conflict analysis
```

---

## LLM Backends

Auto-detection priority: `gemini → kimi → claude → openai → deepseek → bedrock → ollama`

| Backend | Env Var | Default Model | Notes |
|---|---|---|---|
| `claude` | `ANTHROPIC_API_KEY` | `claude-sonnet-4-6` | Best quality |
| `claude-cli` | (local `claude` CLI auth) | `claude-code-plan` | Uses Pro/Max sub, $0 API cost |
| `gemini` | `GEMINI_API_KEY` or `GOOGLE_API_KEY` | `gemini-3-flash-preview` | Fast, cheap |
| `kimi` | `MOONSHOT_API_KEY` | `kimi-k2.6` | Cheapest paid: $0.74/$4.66 per 1M |
| `openai` | `OPENAI_API_KEY` | `gpt-4.1-mini` | Also works with OpenRouter via `OPENAI_BASE_URL` |
| `deepseek` | `DEEPSEEK_API_KEY` | `deepseek-v4-flash` | Cheapest at $0.14/$0.28 per 1M |
| `ollama` | `OLLAMA_API_KEY` (optional) | `qwen2.5-coder:7b` | Fully local, forced serial |
| `bedrock` | AWS credential chain | Claude 3.5 Sonnet | `pip install graphifyy[bedrock]` |

**OpenRouter works:** `OPENAI_API_KEY=*** OPENAI_BASE_URL=https://openrouter.ai/api/v1 graphify extract . --backend openai`

**Env overrides:**
```bash
GRAPHIFY_MAX_OUTPUT_TOKENS=8192    # override max output tokens
GRAPHIFY_API_TIMEOUT=600           # HTTP timeout (default 600s)
GRAPHIFY_OPENAI_MODEL=gpt-4o       # model override for openai backend
GRAPHIFY_GEMINI_MODEL=gemini-2-pro # model override for gemini backend
```

---

## `graph.json` Schema

NetworkX `node_link_data` format. Top-level keys: `nodes`, `links`, `directed`, `hyperedges`, `input_tokens`, `output_tokens`.

### Node fields

```json
{
  "id": "talon_core_agent_loop",
  "label": "Core Agent Loop",
  "source_file": "crates/talon-core/src/agent.rs",
  "source_location": "L42",
  "file_type": "code",
  "community": 7,
  "description": "...",
  "tags": ["...]
}
```

`file_type` values: `code` | `document` | `paper` | `image` | `rationale` | `concept`

### Edge fields

```json
{
  "source": "talon_core_agent_loop",
  "target": "talon_llm_llm_provider",
  "relation": "calls",
  "confidence": "EXTRACTED",
  "confidence_score": 1.0,
  "source_file": "crates/talon-core/src/agent.rs",
  "source_location": "L87",
  "weight": 1.0
}
```

`relation` values: `calls` | `imports` | `imports_from` | `uses` | `references` | `inherits` | `implements` | `mixes_in` | `embeds` | `contains` | `method` | `re_exports` | `conceptually_related_to` | `semantically_similar_to` | `cites` | `shares_data_with`

`confidence` values: `EXTRACTED` (explicit in source) | `INFERRED` (reasoned) | `AMBIGUOUS` (uncertain)

---

## Community Detection

Algorithm: **Leiden** (via `graspologic`) with **Louvain** fallback (NetworkX built-in).

- Oversized communities (>25% of graph) are recursively split by re-running Leiden on the subgraph
- `resolution` parameter: >1.0 = more/smaller communities, <1.0 = fewer/larger
- `--exclude-hubs` flag: suppress utility super-hubs (like `Error`, `String`) from dominating community structure
- Result: every node gets a `community` integer attribute

---

## Output Files

```
graphify-out/
├── graph.json              ← queryable persistent graph (node-link format)
├── graph.html              ← interactive D3/Pyvis visualization
├── GRAPH_REPORT.md         ← god nodes, communities, surprises, suggested questions
├── .graphify_analysis.json ← god nodes + surprises metadata
├── graph.svg               ← static SVG (--svg flag)
├── graph.graphml           ← Gephi/yEd format (--graphml)
├── cypher.txt              ← Neo4j import script (--neo4j)
├── callflow.html           ← Mermaid architecture (graphify export callflow-html)
├── wiki/
│   ├── index.md            ← navigation index for all communities
│   └── community_*.md      ← one page per community
├── obsidian/               ← Obsidian vault with [[wikilinks]] (--obsidian)
└── cache/                  ← semantic extraction cache (for --update incremental)
```

---

## MCP Server

```bash
# Start MCP stdio server (for Claude Code, Cursor, etc.)
graphify . --mcp
# or
graphify serve
```

Exposed MCP tools:
- `query_graph` — BFS/DFS query
- `get_node` — get a single node and its attributes
- `get_neighbors` — get all neighbors of a node
- `shortest_path` — path between two nodes
- `list_prs` / `get_pr_impact` / `triage_prs` — PR integration

---

## Supported Languages (AST extraction, free)

Python, TypeScript, JavaScript, Go, **Rust** ✅, Java, C, C++, Ruby, C#, Kotlin, Scala, PHP, Swift, Lua, Zig, Elixir, Julia, Vue, Svelte, Astro, Groovy, Dart, Fortran, Pascal, Shell, SQL, R, Markdown (structural headings), YAML, and more.

**Markdown:** AST extraction only gets headings/sections. Semantic extraction via LLM gets concepts, relationships, and cross-references. **This is why our inline links and `## Related Documents` sections were so important — they became linkable nodes in the semantic graph.**

---

## Talon-Specific Usage Guide

### Current state (docs phase)

```bash
cd ~/homelab/projects/talon

# Free: update AST graph after any code changes
graphify update .

# View graph
# http://10.10.20.189:7777/graph.html (server running in background)

# Query the graph
graphify query "how does the agent loop interact with tools?"
graphify path "13_Core_Agent_Loop_Design" "41_LLM_Provider_Abstraction"
graphify explain "ToolRegistry"
```

### Semantic enrichment (when you have an API key)

```bash
# Option A: Anthropic (best quality)
export ANTHROPIC_API_KEY="sk-ant-..."
graphify extract . --backend claude

# Option B: Free via OpenRouter
export OPENAI_API_KEY="sk-or-..."
export OPENAI_BASE_URL="https://openrouter.ai/api/v1"
export GRAPHIFY_OPENAI_MODEL="anthropic/claude-3.5-haiku"
graphify extract . --backend openai

# Option C: Fully local
ollama pull qwen2.5-coder:7b
graphify extract . --backend ollama

# Option D: Gemini (free tier available)
export GEMINI_API_KEY="..."
graphify extract . --backend gemini
```

### After Phase 0 (first Rust crates exist)

```bash
# AST extraction on Rust source — free, instant, gives you full call graph
graphify update .

# This will give you:
# - talon_core_agent → talon_llm_llm_provider (calls edge, EXTRACTED)
# - talon_tools_tool → talon_core_tool_registry (implements edge, EXTRACTED)
# - Full community structure matching your crate boundaries
```

### Build the wiki (great for navigating 86 docs)

```bash
graphify . --wiki
# Then browse: graphify-out/wiki/index.md
```

### Generate Obsidian vault

```bash
graphify . --obsidian
# Open graphify-out/obsidian/ in Obsidian
# All docs become interconnected notes with [[wikilinks]]
```

### Export to Neo4j (for complex graph queries)

```bash
graphify . --neo4j                          # generates cypher.txt
# Or push directly:
graphify . --neo4j-push bolt://localhost:7687
# Then: MATCH (n)-[r]->(m) WHERE n.community = 7 RETURN n,r,m
```

---

## Tips & Gotchas

1. **`graphify update .`** only re-runs AST on code files. It does NOT re-process markdown docs. To re-process docs, you need `graphify extract . --backend <llm>` — which costs tokens.

2. **Token budget per query:** Default is 2000 tokens returned. For complex cross-crate queries, use `--budget 4000`.

3. **Node IDs are normalized:** `AgentLoop` in docs becomes `agentloop` or `agent_loop_design` in the graph. Use fuzzy label matching in `path` and `explain` — graphify handles it.

4. **The `wiki/index.md` file** is the best entry point for navigating the graph without looking at `graph.html`. Use it in AI context when you want the agent to understand the graph structure without loading `graph.json`.

5. **`GRAPH_REPORT.md`** only updates when you run a full build or `graphify extract`. It does NOT update on `graphify update .`.

6. **Cache:** Semantic extraction results are cached in `graphify-out/cache/`. If you re-run `extract`, unchanged files aren't re-sent to the LLM. Very cheap to run incremental updates.

7. **`.graphifyignore`:** Create this file (gitignore syntax) to exclude files. Useful for ignoring `graphify-out/` itself, `target/`, `node_modules/`, etc.

```gitignore
# .graphifyignore
graphify-out/
target/
.hermes/
*.lock
```

8. **Git hook:** Run `graphify hook install` once the Rust crates exist. Every `git commit` will automatically run `graphify update .` — the graph stays current without thinking about it.

9. **MCP integration:** Once `graphify . --mcp` is running, Claude Code / Cursor / any MCP client can query the graph as a native tool — no manual `graphify query` needed.

---

## Relationship to Talon Docs

| graphify-out file | Best used for |
|---|---|
| `graph.html` | Visual exploration — see clusters, god nodes, community bridges |
| `GRAPH_REPORT.md` | Broad architecture review — community names, top hub nodes |
| `wiki/index.md` | Navigation when giving AI agents context about the project |
| `graph.json` | Machine-readable — used by `query`, `path`, `explain` commands |
| `callflow.html` | Understanding call sequences and data flow |

The 7 docs in `docs/00_Connections/` complement the graphify graph:
- `00_Cross_Reference_Map.md` — human-readable doc↔community mapping
- `02_Dependency_Order.md` — the DAG that drives the `## Related Documents` sections
- `03_God_Nodes.md` — maps directly to `GRAPH_REPORT.md` god nodes section
- `04_Gap_Analysis.md` — features not yet in graph (because code doesn't exist yet)

---

## Related Documents

### Depends On
- [Phase Build Guide](06_Phase_Build_Guide.md) — when to run which graphify commands per phase
- [Cross-Reference Map](00_Cross_Reference_Map.md) — doc↔community mapping

### See Also
- [God Nodes](03_God_Nodes.md) — top hub nodes in the current graph
- [Gap Analysis](04_Gap_Analysis.md) — what's missing from the graph
- [Workspace & Crate Structure](../02_Architecture/12_Workspace_And_Crate_Structure.md) — how Rust crates will appear as AST nodes
