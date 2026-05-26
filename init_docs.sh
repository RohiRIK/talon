#!/usr/bin/env bash
# Talon AI — docs/ scaffolding script
# Generates all 65 markdown files with proper titles

set -euo pipefail

DOCS_DIR="$(cd "$(dirname "$0")" && pwd)/docs"

create_md() {
  local path="$DOCS_DIR/$1"
  local title="$2"
  mkdir -p "$(dirname "$path")"
  if [[ ! -f "$path" ]]; then
    cat > "$path" <<EOF
# $title

> **Status:** 🚧 Draft
> **Last Updated:** $(date +%Y-%m-%d)

---

<!-- TODO: Fill in content -->
EOF
    echo "  ✓ $path"
  else
    echo "  ⚠ Skipped (exists): $path"
  fi
}

echo "🦀 Talon AI — Initializing docs/ scaffold"
echo "   Target: $DOCS_DIR"
echo ""

# 01_Analysis
create_md "01_Analysis/01_Source_Ecosystem_Overview.md"         "Source Ecosystem Overview"
create_md "01_Analysis/02_OpenClaw_Feature_Audit.md"            "OpenClaw Feature Audit (TypeScript)"
create_md "01_Analysis/03_Hermes_Agent_Feature_Audit.md"        "Hermes Agent Feature Audit (Python)"
create_md "01_Analysis/04_OhMyClaudeCode_Feature_Audit.md"      "oh-my-claudecode Feature Audit"
create_md "01_Analysis/05_Personal_AI_Infra_Feature_Audit.md"   "Personal AI Infrastructure Feature Audit"
create_md "01_Analysis/06_Capability_Matrix.md"                 "Capability Matrix — Keep / Edit / Drop"
create_md "01_Analysis/07_TypeScript_Pain_Points.md"            "TypeScript/Node.js Pain Points & Bottlenecks"
create_md "01_Analysis/08_Python_Pain_Points.md"                "Python Pain Points & Bottlenecks"
create_md "01_Analysis/09_Rust_Migration_Tradeoffs.md"          "Rust Migration Trade-offs Analysis"
create_md "01_Analysis/10_Strategic_Recommendations.md"         "Strategic Recommendations & Guiding Principles"

# 02_Architecture
create_md "02_Architecture/11_System_Architecture_Overview.md"           "Talon System Architecture Overview"
create_md "02_Architecture/12_Workspace_And_Crate_Structure.md"          "Cargo Workspace & Crate Structure"
create_md "02_Architecture/13_Core_Agent_Loop_Design.md"                 "Core Agent Loop Design"
create_md "02_Architecture/14_State_Machine_And_Lifecycle.md"            "State Machine & Agent Lifecycle"
create_md "02_Architecture/15_Context_And_Memory_Architecture.md"        "Context & Memory Architecture"
create_md "02_Architecture/16_Tool_System_Architecture.md"               "Tool System Architecture"
create_md "02_Architecture/17_Plugin_And_Skill_Architecture.md"          "Plugin & Skill Architecture"
create_md "02_Architecture/18_Gateway_MultiChannel_Architecture.md"      "Gateway & Multi-Channel Architecture"
create_md "02_Architecture/19_Subagent_And_Delegation_Architecture.md"   "Subagent & Delegation Architecture"
create_md "02_Architecture/20_Security_Model.md"                         "Security Model & Trust Boundaries"

# 03_Migration_Strategy
create_md "03_Migration_Strategy/21_Migration_Roadmap.md"               "Migration Roadmap & Phases"
create_md "03_Migration_Strategy/22_TypeScript_To_Rust_Patterns.md"     "TypeScript-to-Rust Migration Patterns"
create_md "03_Migration_Strategy/23_Python_To_Rust_Patterns.md"         "Python-to-Rust Migration Patterns"
create_md "03_Migration_Strategy/24_Async_Migration_NodeJS_To_Tokio.md" "Async Migration: Node.js → Tokio"
create_md "03_Migration_Strategy/25_Data_Model_Migration.md"            "Data Model Migration"
create_md "03_Migration_Strategy/26_Test_Strategy.md"                   "Test Strategy & Coverage Targets"
create_md "03_Migration_Strategy/27_Incremental_Migration_Approach.md"  "Incremental Migration & Interop Strategy"
create_md "03_Migration_Strategy/28_Risk_Register.md"                   "Risk Register & Mitigation Playbook"

# 04_Core_Features
create_md "04_Core_Features/29_Agent_Loop_Implementation.md"          "Agent Loop Implementation"
create_md "04_Core_Features/30_Tool_Execution_Engine.md"              "Tool Execution Engine"
create_md "04_Core_Features/31_Streaming_And_Realtime_Output.md"      "Streaming & Real-time Output"
create_md "04_Core_Features/32_Session_And_Conversation_Management.md" "Session & Conversation Management"
create_md "04_Core_Features/33_Cron_Scheduler.md"                     "Cron Scheduler & Job Management"
create_md "04_Core_Features/34_Skill_System.md"                       "Skill System"
create_md "04_Core_Features/35_Memory_System_SQLite_FTS5.md"          "Memory System (SQLite + FTS5)"
create_md "04_Core_Features/36_TUI_Implementation.md"                 "TUI Implementation (Ratatui)"
create_md "04_Core_Features/37_Voice_Mode.md"                         "Voice Mode"
create_md "04_Core_Features/38_Batch_Trajectory_Generation.md"        "Batch Trajectory Generation"
create_md "04_Core_Features/39_Self_Evolution_Loop.md"                "Self-Evolution Loop"
create_md "04_Core_Features/40_Profile_Isolation.md"                  "Profile Isolation"

# 05_API_Bindings
create_md "05_API_Bindings/41_LLM_Provider_Abstraction.md"       "LLM Provider Abstraction Layer"
create_md "05_API_Bindings/42_OpenAI_Compatible_Client.md"        "OpenAI-Compatible API Client"
create_md "05_API_Bindings/43_Anthropic_API_Integration.md"       "Anthropic API Integration"
create_md "05_API_Bindings/44_Messaging_Platform_Gateway.md"      "Messaging Platform Gateway Design"
create_md "05_API_Bindings/45_Telegram_Integration.md"            "Telegram Bot Integration"
create_md "05_API_Bindings/46_Discord_Integration.md"             "Discord Integration"
create_md "05_API_Bindings/47_MCP_Protocol_Integration.md"        "MCP Protocol Integration"
create_md "05_API_Bindings/48_ACP_Protocol_Integration.md"        "ACP Protocol Integration"

# 06_Concurrency
create_md "06_Concurrency/49_Tokio_Runtime_Design.md"             "Tokio Runtime Design & Configuration"
create_md "06_Concurrency/50_Async_Tool_Execution.md"             "Async Tool Execution Patterns"
create_md "06_Concurrency/51_Parallel_Subagent_Spawning.md"       "Parallel Subagent Spawning"
create_md "06_Concurrency/52_Stream_Processing.md"                "Stream Processing"
create_md "06_Concurrency/53_Resource_Limits_And_Backpressure.md" "Resource Limits & Backpressure"
create_md "06_Concurrency/54_Error_Handling_Strategy.md"          "Error Handling Strategy"

# 07_Memory_System
create_md "07_Memory_System/55_SQLite_FTS5_In_Rust.md"         "SQLite + FTS5 in Rust"
create_md "07_Memory_System/56_Cross_Session_Context.md"        "Cross-Session Context Persistence"
create_md "07_Memory_System/57_Skill_File_Management.md"        "Skill File Management & Hot-Reload"
create_md "07_Memory_System/58_User_Modeling.md"                "User Modeling & Profile Persistence"
create_md "07_Memory_System/59_Embedding_Based_Retrieval.md"    "Embedding-based Semantic Retrieval"

# 08_DevOps
create_md "08_DevOps/60_Build_System_Cargo_Workspace.md"      "Build System & Cargo Workspace Config"
create_md "08_DevOps/61_Docker_And_Container_Deployment.md"   "Docker & Container Deployment"
create_md "08_DevOps/62_CI_CD_Pipeline.md"                    "CI/CD Pipeline (GitHub Actions)"
create_md "08_DevOps/63_Configuration_Management.md"          "Configuration Management"
create_md "08_DevOps/64_Logging_And_Observability.md"         "Logging & Observability"
create_md "08_DevOps/65_Release_And_Distribution.md"          "Release & Distribution Strategy"

echo ""
echo "✅ Scaffold complete — 65 documents created under $DOCS_DIR"
