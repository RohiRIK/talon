# Messaging Platform Gateway

> **Status:** ✅ Complete
> **Category:** API Bindings

---

## 1. Overview

The messaging gateway layer abstracts platform-specific APIs
(Telegram, Discord, Slack, Signal) behind the common `Gateway` trait.

See doc `18_Gateway_MultiChannel_Architecture.md` for the full trait definition.

This document covers implementation details for each platform adapter.

---

## 2. Platform Support Matrix

| Platform | Status | Crate | Notes |
|----------|--------|-------|-------|
| Telegram | ✅ | `[teloxide](45_Telegram_Integration.md)` | Full support, voice, inline keyboard |
| Discord | ✅ | `serenity` or `twilight` | Text + slash commands |
| CLI (local) | ✅ | `[ratatui](../04_Core_Features/36_TUI_Implementation.md)` / stdout | TUI or plain print |
| HTTP API | ✅ | `axum` | REST + SSE |
| Slack | 🔧 | `slack-morphism` | Planned |
| Signal | ❌ | — | No good Rust lib |
| Matrix | 🔧 | `matrix-sdk` | Planned |

---

## 3. Message Format Normalization

Each platform has its own markdown/formatting syntax.
Talon normalizes output before delivery:

```rust
pub fn normalize_for_platform(text: &str, platform: &str) -> String {
    match platform {
        "telegram" => markdown_to_telegram_html(text),
        "discord"  => markdown_to_discord(text),
        "slack"    => markdown_to_slack_mrkdwn(text),
        _          => text.to_string(),  // plain text for CLI/HTTP
    }
}

fn markdown_to_telegram_html(md: &str) -> String {
    // Telegram MarkdownV2 escaping is fragile — use HTML mode instead
    // **bold** → <b>bold</b>
    // *italic* → <i>italic</i>
    // `code` → <code>code</code>
    // ```\nblock\n``` → <pre>block</pre>
    // [text](url) → <a href="url">text</a>
    // Tables → escaped as bullet lists (Telegram has no table support)
    use regex::Regex;
    let mut out = md.to_string();
    // Bold
    out = Regex::new(r"\*\*(.+?)\*\*").unwrap()
        .replace_all(&out, "<b>$1</b>").to_string();
    // Code blocks (must come before inline code)
    out = Regex::new(r"```[\w]*\n([\s\S]+?)\n```").unwrap()
        .replace_all(&out, "<pre>$1</pre>").to_string();
    // Inline code
    out = Regex::new(r"`(.+?)`").unwrap()
        .replace_all(&out, "<code>$1</code>").to_string();
    out
}
```

---

## 4. Message Splitting

Platforms have character limits:
- Telegram: 4096 chars
- Discord: 2000 chars
- Slack: 3001 chars (mrkdwn blocks)

Talon splits long messages at paragraph boundaries:

```rust
pub fn split_message(text: &str, max_chars: usize) -> Vec<String> {
    if text.len() <= max_chars {
        return vec![text.to_string()];
    }

    let mut parts = vec![];
    let mut current = String::new();

    for paragraph in text.split("\n\n") {
        if current.len() + paragraph.len() + 2 > max_chars {
            if !current.is_empty() {
                parts.push(current.trim_end().to_string());
                current = String::new();
            }
            // Paragraph itself is over limit: hard-split
            if paragraph.len() > max_chars {
                for chunk in paragraph.as_bytes().chunks(max_chars) {
                    parts.push(String::from_utf8_lossy(chunk).to_string());
                }
                continue;
            }
        }
        if !current.is_empty() { current.push_str("\n\n"); }
        current.push_str(paragraph);
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}
```

---

## 5. Delivery Receipts & Error Handling

```rust
pub async fn deliver_with_retry(
    gateway: &dyn Gateway,
    event: &DeliveryEvent,
    max_retries: u32,
) -> Result<(), GatewayError> {
    let mut last_error = None;

    for attempt in 0..=max_retries {
        match gateway.deliver(event).await {
            Ok(()) => return Ok(()),
            Err(GatewayError::RateLimited { retry_after }) => {
                tokio::time::sleep(retry_after).await;
            }
            Err(GatewayError::MessageTooLong) => {
                // Non-retryable: should have been split before delivery
                return Err(GatewayError::MessageTooLong);
            }
            Err(e) => {
                if attempt < max_retries {
                    tokio::time::sleep(Duration::from_secs(2u64.pow(attempt))).await;
                }
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap())
}
```
---

## Related Documents

### Depends On
- [Gateway Architecture](../02_Architecture/18_Gateway_MultiChannel_Architecture.md)

### See Also
- [Telegram Integration](45_Telegram_Integration.md)
- [Discord Integration](46_Discord_Integration.md)

