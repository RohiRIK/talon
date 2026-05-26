# Browser Tool Implementation

> **Status:** ✅ Complete
> **Category:** Core Features
> **Last corrected:** dogfood pass 3

---

## 1. Architecture Overview

The browser tool drives a real Chromium instance via Chrome DevTools Protocol (CDP).
Talon does NOT depend on Playwright or Puppeteer — it uses the `chromiumoxide` crate
(pure Rust, async CDP client).

```
BrowserTool
    │
    ├── BrowserPool (manages Chrome instances)
    │       └── chromiumoxide::Browser
    │
    ├── PageSession (per-agent, per-tab)
    │       ├── chromiumoxide::Page
    │       └── SnapshotBuilder → AccessibilityTree
    │
    └── VisionAnalyzer (optional — base64 screenshot → LLM vision)
```

---

## 2. Tool Definitions

Five distinct sub-tools map from OpenClaw/Hermes browser capabilities:

| Tool | Purpose |
|------|---------|
| `browser_navigate` | Open URL, return accessibility snapshot |
| `browser_snapshot` | Get current page accessibility tree |
| `browser_click` | Click element by ref ID |
| `browser_type` | Type text into input by ref ID |
| `browser_vision` | Screenshot → vision analysis |

These are separate `Tool` implementations sharing a `BrowserPool`.

---

## 3. BrowserPool

```rust
pub struct BrowserPool {
    browsers: Mutex<Vec<BrowserInstance>>,
    max_instances: usize,
    chrome_path: PathBuf,
    launch_args: Vec<String>,
}

struct BrowserInstance {
    browser: chromiumoxide::Browser,
    task: JoinHandle<()>,  // event loop task
    in_use: bool,
}

impl BrowserPool {
    pub async fn acquire(&self) -> Result<BrowserGuard, BrowserError> {
        let mut pool = self.browsers.lock().await;

        // Find idle instance
        if let Some(inst) = pool.iter_mut().find(|b| !b.in_use) {
            inst.in_use = true;
            return Ok(BrowserGuard { /* ... */ });
        }

        // Spawn new if under limit
        if pool.len() < self.max_instances {
            let (browser, handler) = chromiumoxide::Browser::launch(
                chromiumoxide::BrowserConfig::builder()
                    .chrome_executable(&self.chrome_path)
                    .arg("--headless=new")
                    .arg("--no-sandbox")
                    .arg("--disable-gpu")
                    .args(&self.launch_args)
                    .build()?,
            )
            .await?;

            let task = tokio::spawn(async move {
                let mut handler = handler;
                while handler.next().await.is_some() {}
            });

            pool.push(BrowserInstance { browser, task, in_use: true });
            return Ok(BrowserGuard { /* ... */ });
        }

        Err(BrowserError::PoolExhausted)
    }
}
```

---

## 4. Accessibility Snapshot

The key innovation: convert the CDP accessibility tree into a compact text representation
with stable `@eN` ref IDs that the LLM can reference in subsequent tool calls.

```rust
pub struct AccessibilitySnapshot {
    pub text: String,              // The compact tree sent to LLM
    pub ref_map: HashMap<u32, NodeRef>,  // @e5 → CDP node ID + frame ID
}

pub struct SnapshotBuilder {
    counter: u32,
    nodes: Vec<SnapshotNode>,
    ref_map: HashMap<u32, NodeRef>,
}

impl SnapshotBuilder {
    pub async fn build(&mut self, page: &chromiumoxide::Page) -> Result<AccessibilitySnapshot, BrowserError> {
        let tree = page.accessibility_tree(None).await?;
        self.counter = 0;
        self.nodes.clear();
        self.ref_map.clear();

        self.visit_node(&tree, 0);

        let text = self.render();
        Ok(AccessibilitySnapshot {
            text,
            ref_map: self.ref_map.clone(),
        })
    }

    fn visit_node(&mut self, node: &AXNode, depth: usize) {
        let role = node.role.as_deref().unwrap_or("unknown");
        let name = node.name.as_deref().unwrap_or("");

        // Only include interactive or content-bearing nodes
        if !is_relevant(role) { return; }

        let ref_id = if is_interactive(role) {
            self.counter += 1;
            let id = self.counter;
            self.ref_map.insert(id, NodeRef {
                node_id: node.node_id,
                backend_id: node.backend_dom_node_id,
            });
            Some(id)
        } else {
            None
        };

        let indent = "  ".repeat(depth);
        let ref_str = ref_id
            .map(|id| format!(" [@e{id}]"))
            .unwrap_or_default();

        self.nodes.push(SnapshotNode {
            line: format!("{indent}{role} \"{name}\"{ref_str}"),
        });

        for child in &node.children {
            self.visit_node(child, depth + 1);
        }
    }
}

fn is_interactive(role: &str) -> bool {
    matches!(role,
        "button" | "link" | "textbox" | "combobox" | "checkbox"
        | "radio" | "listbox" | "option" | "menuitem" | "tab"
        | "searchbox" | "spinbutton" | "slider"
    )
}
```

---

## 5. BrowserNavigate Tool

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowserNavigateParams {
    pub url: String,
}

#[async_trait]
impl Tool for BrowserNavigateTool {
    fn name(&self) -> &str { "browser_navigate" }
    fn approval_level(&self) -> ApprovalLevel { ApprovalLevel::Confirmation }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let p: BrowserNavigateParams = serde_json::from_value(args)?;

        let session = ctx.browser_session.get_or_create().await?;

        // Navigate with timeout
        tokio::time::timeout(Duration::from_secs(30), async {
            session.page.goto(&p.url).await?;
            session.page.wait_for_navigation().await
        })
        .await
        .map_err(|_| BrowserError::NavigationTimeout)?
        .map_err(BrowserError::Cdp)?;

        // Build snapshot
        let snapshot = session.snapshot_builder.build(&session.page).await?;

        // Cache ref_map for subsequent click/type calls
        ctx.browser_session.update_ref_map(snapshot.ref_map).await;

        Ok(ToolResult::text(snapshot.text))
    }
}
```

---

## 6. BrowserClick Tool

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowserClickParams {
    /// Element reference from snapshot (e.g., "@e5")
    pub ref_id: String,
}

#[async_trait]
impl Tool for BrowserClickTool {
    fn name(&self) -> &str { "browser_click" }
    fn approval_level(&self) -> ApprovalLevel { ApprovalLevel::Confirmation }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let p: BrowserClickParams = serde_json::from_value(args)?;

        // Parse "@e5" → 5
        let n: u32 = p.ref_id
            .strip_prefix("@e")
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| ToolError::InvalidParams(
                format!("Invalid ref_id '{}' — expected format @eN", p.ref_id)
            ))?;

        let session = ctx.browser_session.get().await
            .ok_or(BrowserError::NoActivePage)?;

        let node_ref = session.ref_map.get(&n)
            .ok_or_else(|| BrowserError::RefNotFound(p.ref_id.clone()))?;

        session.page
            .find_element(format!("[data-cdp-id='{}']", node_ref.node_id))
            .await
            .map_err(|_| BrowserError::ElementNotFound(p.ref_id.clone()))?
            .click()
            .await
            .map_err(BrowserError::Cdp)?;

        // Re-snapshot after click (DOM may have changed)
        tokio::time::sleep(Duration::from_millis(500)).await;
        let snapshot = session.snapshot_builder.build(&session.page).await?;
        ctx.browser_session.update_ref_map(snapshot.ref_map).await;

        Ok(ToolResult::text(snapshot.text))
    }
}
```

---

## 7. BrowserVision Tool

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowserVisionParams {
    pub question: String,
    /// Overlay ref labels on screenshot (default: false)
    #[serde(default)]
    pub annotate: bool,
}

#[async_trait]
impl Tool for BrowserVisionTool {
    fn name(&self) -> &str { "browser_vision" }
    fn approval_level(&self) -> ApprovalLevel { ApprovalLevel::Confirmation }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let p: BrowserVisionParams = serde_json::from_value(args)?;

        let session = ctx.browser_session.get().await
            .ok_or(BrowserError::NoActivePage)?;

        let screenshot = session.page
            .screenshot(
                chromiumoxide::page::ScreenshotParams::builder()
                    .format(chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png)
                    .build(),
            )
            .await
            .map_err(BrowserError::Cdp)?;

        // Save screenshot to temp file
        let path = ctx.temp_dir.join(format!("screenshot_{}.png", Uuid::new_v4()));
        tokio::fs::write(&path, &screenshot).await.map_err(ToolError::Io)?;

        // Ask vision model
        let vision_response = ctx.llm
            .vision_complete(VisionRequest {
                image_path: path.clone(),
                question: p.question,
                model: ctx.config.vision_model.clone(),
            })
            .await
            .map_err(ToolError::Llm)?;

        Ok(ToolResult::with_media(
            vision_response.text,
            MediaAttachment::Image(path),
        ))
    }
}
```

---

## 8. Session Lifecycle

```
browse_navigate("https://example.com")
    → Creates PageSession, navigates, returns snapshot + @e refs cached

browser_click("@e3")
    → Resolves @e3 → CDP node ID from cache, clicks, returns new snapshot

browser_type("@e5", "Hello")
    → Types into input field, returns updated snapshot

browser_vision("What is on this page?")
    → Screenshots, sends to vision LLM, returns text + screenshot path

// Session auto-closes after:
// - 30 minutes idle
// - Agent session ends
// - Explicit browser_close call
```

---

## 9. Error Handling

```rust
#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    #[error("Browser pool exhausted — max {0} instances")]
    PoolExhausted(usize),
    #[error("Navigation timeout after 30s for URL: {0}")]
    NavigationTimeout(String),
    #[error("Element ref '{0}' not found in current snapshot")]
    RefNotFound(String),
    #[error("No active page — call browser_navigate first")]
    NoActivePage,
    #[error("CDP error: {0}")]
    Cdp(#[from] chromiumoxide::error::CdpError),
    #[error("Chrome not found at {0} — install Chromium")]
    ChromeNotFound(PathBuf),
}
```

> **Last corrected:** dogfood pass 4
---

## Related Documents

### Depends On
- [Tool System Architecture](../02_Architecture/16_Tool_System_Architecture.md)

### See Also
- [Web Search Tool](34_Web_Search_Tool.md)
- [Approval Membrane](../02_Architecture/17a_Approval_Membrane.md)

