/// Per-invocation approval classification. Computed with actual tool arguments,
/// not as a static property of the tool — prevents tools lying about danger level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalLevel {
    Safe,
    NeedsApproval,
    Dangerous,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_level_equality() {
        assert_eq!(ApprovalLevel::Safe, ApprovalLevel::Safe);
        assert_ne!(ApprovalLevel::Safe, ApprovalLevel::Dangerous);
    }

    #[test]
    fn approval_level_copy() {
        let a = ApprovalLevel::NeedsApproval;
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn approval_level_debug() {
        assert_eq!(format!("{:?}", ApprovalLevel::Dangerous), "Dangerous");
    }

    #[test]
    fn all_three_variants_are_distinct() {
        assert_ne!(ApprovalLevel::Safe, ApprovalLevel::NeedsApproval);
        assert_ne!(ApprovalLevel::NeedsApproval, ApprovalLevel::Dangerous);
        assert_ne!(ApprovalLevel::Safe, ApprovalLevel::Dangerous);
    }

    #[test]
    fn pattern_match_safe() {
        let level = ApprovalLevel::Safe;
        let auto_approved = matches!(level, ApprovalLevel::Safe | ApprovalLevel::NeedsApproval);
        assert!(auto_approved);
    }

    #[test]
    fn pattern_match_dangerous_requires_gate() {
        let level = ApprovalLevel::Dangerous;
        let needs_gate = matches!(level, ApprovalLevel::Dangerous);
        assert!(needs_gate);
    }
}
