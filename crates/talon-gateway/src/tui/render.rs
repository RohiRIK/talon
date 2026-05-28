use crate::RenderMode;

/// Detect the best render mode for the current terminal environment.
///
/// Priority (first match wins):
/// 1. `NO_COLOR` set → `Plain`
/// 2. `TERM=dumb` → `Plain`
/// 3. Stdin is piped (not a TTY) → `Plain`
/// 4. `--accessible` / `TALON_ACCESSIBLE` set → `Accessible`
/// 5. Otherwise → `Tui`
pub fn detect_capabilities() -> RenderMode {
    // NO_COLOR — honour this convention universally.
    if std::env::var("NO_COLOR").is_ok() {
        return RenderMode::Plain;
    }

    // Dumb terminal (CI runners, basic SSH sessions).
    if std::env::var("TERM").as_deref() == Ok("dumb") {
        return RenderMode::Plain;
    }

    // Piped stdin — we're not interactive.
    if !is_tty() {
        return RenderMode::Plain;
    }

    // Explicit accessibility request.
    if std::env::var("TALON_ACCESSIBLE").is_ok() {
        return RenderMode::Accessible;
    }

    RenderMode::Tui
}

/// Override the detected mode if the user passed `--accessible`.
pub fn apply_accessible_flag(mode: RenderMode, accessible: bool) -> RenderMode {
    if accessible { RenderMode::Accessible } else { mode }
}

fn is_tty() -> bool {
    // Use the crossterm helper — more reliable than checking STDOUT on its own.
    crossterm::terminal::is_raw_mode_enabled().is_ok()
        || std::io::IsTerminal::is_terminal(&std::io::stdin())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_returns_plain() {
        // Temporarily set NO_COLOR; restore afterward.
        unsafe {
            std::env::set_var("NO_COLOR", "1");
        }
        let mode = detect_capabilities();
        unsafe {
            std::env::remove_var("NO_COLOR");
        }
        assert_eq!(mode, RenderMode::Plain);
    }

    #[test]
    fn dumb_term_returns_plain() {
        let prev = std::env::var("TERM").ok();
        unsafe {
            std::env::set_var("TERM", "dumb");
        }
        let mode = detect_capabilities();
        match prev {
            Some(v) => unsafe { std::env::set_var("TERM", v) },
            None => unsafe { std::env::remove_var("TERM") },
        }
        assert_eq!(mode, RenderMode::Plain);
    }

    #[test]
    fn accessible_flag_overrides_tui() {
        assert_eq!(apply_accessible_flag(RenderMode::Tui, true), RenderMode::Accessible);
        assert_eq!(apply_accessible_flag(RenderMode::Tui, false), RenderMode::Tui);
    }

    #[test]
    fn accessible_flag_preserves_plain() {
        assert_eq!(apply_accessible_flag(RenderMode::Plain, true), RenderMode::Accessible);
    }
}
