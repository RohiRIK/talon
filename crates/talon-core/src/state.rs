use crate::error::CoreError;

/// State machine for one agent run.
/// Transitions: Idle → Thinking → CallingTool → Thinking (loop) → Completed | Failed
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentState {
    Idle,
    Thinking,
    CallingTool { tool_name: String },
    Completed,
    Failed(String),
}

impl AgentState {
    pub fn transition(&self, next: AgentState) -> Result<AgentState, CoreError> {
        let valid = matches!(
            (&self, &next),
            (AgentState::Idle, AgentState::Thinking)
                | (AgentState::Thinking, AgentState::CallingTool { .. })
                | (AgentState::CallingTool { .. }, AgentState::Thinking)
                | (AgentState::Thinking, AgentState::Completed)
                | (AgentState::Thinking, AgentState::Failed(_))
                | (AgentState::CallingTool { .. }, AgentState::Failed(_))
        );
        if valid {
            Ok(next)
        } else {
            Err(CoreError::InvalidState(format!(
                "cannot transition from {self:?} to {next:?}"
            )))
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, AgentState::Completed | AgentState::Failed(_))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn idle_to_thinking_is_valid() {
        let s = AgentState::Idle;
        let next = s.transition(AgentState::Thinking).expect("valid");
        assert_eq!(next, AgentState::Thinking);
    }

    #[test]
    fn thinking_to_calling_tool_is_valid() {
        let s = AgentState::Thinking;
        let next = s
            .transition(AgentState::CallingTool { tool_name: "echo".to_string() })
            .expect("valid");
        assert!(matches!(next, AgentState::CallingTool { .. }));
    }

    #[test]
    fn calling_tool_back_to_thinking_is_valid() {
        let s = AgentState::CallingTool { tool_name: "echo".to_string() };
        let next = s.transition(AgentState::Thinking).expect("valid");
        assert_eq!(next, AgentState::Thinking);
    }

    #[test]
    fn thinking_to_completed_is_valid() {
        let s = AgentState::Thinking;
        let next = s.transition(AgentState::Completed).expect("valid");
        assert_eq!(next, AgentState::Completed);
    }

    #[test]
    fn thinking_to_failed_is_valid() {
        let s = AgentState::Thinking;
        let next = s.transition(AgentState::Failed("oops".to_string())).expect("valid");
        assert!(matches!(next, AgentState::Failed(_)));
    }

    #[test]
    fn idle_to_completed_is_invalid() {
        let s = AgentState::Idle;
        assert!(s.transition(AgentState::Completed).is_err());
    }

    #[test]
    fn completed_to_thinking_is_invalid() {
        let s = AgentState::Completed;
        assert!(s.transition(AgentState::Thinking).is_err());
    }

    #[test]
    fn invalid_transition_error_contains_state_names() {
        let s = AgentState::Idle;
        let err = s.transition(AgentState::Completed).unwrap_err();
        assert!(err.to_string().contains("Idle"));
        assert!(err.to_string().contains("Completed"));
    }

    #[test]
    fn completed_is_terminal() {
        assert!(AgentState::Completed.is_terminal());
    }

    #[test]
    fn failed_is_terminal() {
        assert!(AgentState::Failed("x".to_string()).is_terminal());
    }

    #[test]
    fn idle_is_not_terminal() {
        assert!(!AgentState::Idle.is_terminal());
    }

    #[test]
    fn thinking_is_not_terminal() {
        assert!(!AgentState::Thinking.is_terminal());
    }

    #[test]
    fn calling_tool_is_not_terminal() {
        assert!(!AgentState::CallingTool { tool_name: "x".to_string() }.is_terminal());
    }

    #[test]
    fn idle_to_calling_tool_is_invalid() {
        let s = AgentState::Idle;
        assert!(s.transition(AgentState::CallingTool { tool_name: "t".to_string() }).is_err());
    }

    #[test]
    fn idle_to_failed_is_invalid() {
        let s = AgentState::Idle;
        assert!(s.transition(AgentState::Failed("x".to_string())).is_err());
    }

    #[test]
    fn calling_tool_to_completed_is_invalid() {
        let s = AgentState::CallingTool { tool_name: "t".to_string() };
        assert!(s.transition(AgentState::Completed).is_err());
    }

    #[test]
    fn failed_to_thinking_is_invalid() {
        let s = AgentState::Failed("oops".to_string());
        assert!(s.transition(AgentState::Thinking).is_err());
    }

    #[test]
    fn completed_to_failed_is_invalid() {
        let s = AgentState::Completed;
        assert!(s.transition(AgentState::Failed("x".to_string())).is_err());
    }
}
