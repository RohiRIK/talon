use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("LLM error: {0}")]
    Llm(String),

    #[error("tool error in '{tool}': {message}")]
    Tool { tool: String, message: String },

    #[error("approval denied for tool '{tool}'")]
    ApprovalDenied { tool: String },

    #[error("operation timed out after {secs}s")]
    Timeout { secs: u64 },

    #[error("invalid state: {0}")]
    InvalidState(String),
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn llm_error_display() {
        let e = CoreError::Llm("rate limited".to_string());
        assert_eq!(e.to_string(), "LLM error: rate limited");
    }

    #[test]
    fn tool_error_display() {
        let e = CoreError::Tool {
            tool: "read_file".to_string(),
            message: "file not found".to_string(),
        };
        assert_eq!(e.to_string(), "tool error in 'read_file': file not found");
    }

    #[test]
    fn approval_denied_display() {
        let e = CoreError::ApprovalDenied {
            tool: "delete_file".to_string(),
        };
        assert_eq!(e.to_string(), "approval denied for tool 'delete_file'");
    }

    #[test]
    fn timeout_display() {
        let e = CoreError::Timeout { secs: 30 };
        assert_eq!(e.to_string(), "operation timed out after 30s");
    }

    #[test]
    fn invalid_state_display() {
        let e = CoreError::InvalidState("expected Idle, got Thinking".to_string());
        assert_eq!(e.to_string(), "invalid state: expected Idle, got Thinking");
    }

    #[test]
    fn core_error_is_debug() {
        let e = CoreError::Llm("test".to_string());
        assert!(format!("{e:?}").contains("Llm"));
    }

    #[test]
    fn core_error_implements_std_error() {
        let e: Box<dyn std::error::Error> = Box::new(CoreError::Timeout { secs: 60 });
        assert!(e.to_string().contains("60"));
    }
}
