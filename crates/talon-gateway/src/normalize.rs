use crate::RenderMode;

/// Normalize markdown for the target render mode.
///
/// - `Tui`        → pass through; ratatui components render raw markdown.
/// - `Accessible` → strip formatting, preserve structure as plain lines.
/// - `Plain`      → strip all markdown, single stream of text.
pub fn normalize_markdown(content: &str, mode: RenderMode) -> String {
    match mode {
        RenderMode::Tui => content.to_string(),
        RenderMode::Accessible => to_accessible(content),
        RenderMode::Plain => to_plain(content),
    }
}

/// Strip inline markdown markers; keep structural cues (blank lines, bullets).
fn to_accessible(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut in_code_block = false;

    for line in content.lines() {
        if line.starts_with("```") {
            in_code_block = !in_code_block;
            if in_code_block {
                out.push_str("[code]\n");
            } else {
                out.push_str("[/code]\n");
            }
            continue;
        }

        if in_code_block {
            out.push_str(line);
            out.push('\n');
            continue;
        }

        // Headings → capitalised text without #
        let stripped = if let Some(rest) = line.strip_prefix("### ") {
            rest.to_uppercase()
        } else if let Some(rest) = line.strip_prefix("## ") {
            rest.to_uppercase()
        } else if let Some(rest) = line.strip_prefix("# ") {
            rest.to_uppercase()
        } else {
            strip_inline(line)
        };

        out.push_str(&stripped);
        out.push('\n');
    }

    out
}

/// Strip all markdown markers, returning plain prose.
fn to_plain(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut in_code_block = false;

    for line in content.lines() {
        if line.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        // Strip heading markers
        let line = line
            .trim_start_matches("### ")
            .trim_start_matches("## ")
            .trim_start_matches("# ");

        // Strip list markers
        let line = line
            .trim_start_matches("- ")
            .trim_start_matches("* ")
            .trim_start_matches("+ ");

        let line = if in_code_block {
            line.to_string()
        } else {
            strip_inline(line)
        };

        if !line.is_empty() {
            out.push_str(&line);
            out.push('\n');
        }
    }

    out
}

/// Strip inline markers: bold, italic, inline code, strikethrough.
fn strip_inline(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            // Bold/italic: ** or *
            '*' => {
                // Skip **, ***, or *
                let stars = chars[i..].iter().take_while(|&&c| c == '*').count();
                i += stars;
            }
            // Bold/italic: __ or _
            '_' => {
                let underscores = chars[i..].iter().take_while(|&&c| c == '_').count();
                i += underscores;
            }
            // Inline code
            '`' => {
                i += 1;
                while i < chars.len() && chars[i] != '`' {
                    out.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    i += 1; // closing `
                }
            }
            // Strikethrough ~~
            '~' if i + 1 < chars.len() && chars[i + 1] == '~' => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '~' && chars[i + 1] == '~') {
                    out.push(chars[i]);
                    i += 1;
                }
                i += 2; // closing ~~
            }
            // Links: [text](url) → text
            '[' => {
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != ']' {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                out.push_str(&text);
                if i < chars.len() {
                    i += 1; // ]
                }
                // Skip (url) if present
                if i < chars.len() && chars[i] == '(' {
                    i += 1;
                    while i < chars.len() && chars[i] != ')' {
                        i += 1;
                    }
                    if i < chars.len() {
                        i += 1;
                    }
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }

    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tui_passthrough() {
        let md = "**bold** and _italic_";
        assert_eq!(normalize_markdown(md, RenderMode::Tui), md);
    }

    #[test]
    fn plain_strips_bold() {
        let result = normalize_markdown("**hello** world", RenderMode::Plain);
        assert!(!result.contains('*'));
        assert!(result.contains("hello"));
        assert!(result.contains("world"));
    }

    #[test]
    fn plain_strips_heading() {
        let result = normalize_markdown("# My Title\nsome text", RenderMode::Plain);
        assert!(!result.contains('#'));
        assert!(result.contains("My Title"));
    }

    #[test]
    fn plain_strips_code_fence_markers() {
        let result = normalize_markdown("```rust\nfn main() {}\n```", RenderMode::Plain);
        assert!(!result.contains("```"));
        assert!(result.contains("fn main()"));
    }

    #[test]
    fn plain_extracts_link_text() {
        let result = normalize_markdown("[click here](https://example.com)", RenderMode::Plain);
        assert!(result.contains("click here"));
        assert!(!result.contains("https://"));
    }

    #[test]
    fn accessible_marks_code_blocks() {
        let result =
            normalize_markdown("```rust\nlet x = 1;\n```", RenderMode::Accessible);
        assert!(result.contains("[code]"));
        assert!(result.contains("[/code]"));
        assert!(result.contains("let x = 1;"));
    }

    #[test]
    fn accessible_uppercases_headings() {
        let result = normalize_markdown("## Section Title", RenderMode::Accessible);
        assert!(result.contains("SECTION TITLE"));
    }

    #[test]
    fn plain_inline_code_preserved() {
        let result = normalize_markdown("use `cargo nextest`", RenderMode::Plain);
        assert!(result.contains("cargo nextest"));
        assert!(!result.contains('`'));
    }

    #[test]
    fn plain_empty_lines_stripped() {
        let result = normalize_markdown("\n\n\ntext\n\n", RenderMode::Plain);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines, vec!["text"]);
    }
}
