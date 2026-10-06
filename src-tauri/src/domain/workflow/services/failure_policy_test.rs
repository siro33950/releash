pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn timeout_policy_uses_defaults_and_template_overrides() {
        let policy = TimeoutPolicy::default()
            .with_stale_timeout_for_template("heavy-review", Duration::from_secs(600))
            .with_stale_timeout_for_model("slow-model", Duration::from_secs(480));

        assert_eq!(
            policy.startup_timeout(&TimeoutContext::default()),
            Duration::from_secs(30)
        );
        assert_eq!(
            policy.stale_timeout(&TimeoutContext::default()),
            Duration::from_secs(180)
        );
        assert_eq!(
            policy.stale_timeout(&TimeoutContext::new(
                None,
                NodeKindName::Session,
                Some("heavy-review".to_string())
            )),
            Duration::from_secs(600)
        );
        assert_eq!(
            policy.stale_timeout(&TimeoutContext::new(
                Some("slow-model".to_string()),
                NodeKindName::Session,
                None
            )),
            Duration::from_secs(480)
        );
    }

    #[test]
    fn timeout_policy_resolves_node_kind_only_override() {
        let policy = TimeoutPolicy::default()
            .with_stale_timeout_for_node_kind(NodeKindName::Session, Duration::from_secs(420))
            .with_stale_timeout_for_model("slow-model", Duration::from_secs(480))
            .with_stale_timeout_for_template("heavy-review", Duration::from_secs(600));

        assert_eq!(
            policy.stale_timeout(&TimeoutContext::new(
                Some("unknown-model".to_string()),
                NodeKindName::Session,
                Some("unknown-template".to_string())
            )),
            Duration::from_secs(420)
        );
    }

    #[test]
    fn timeout_policy_prefers_template_over_node_kind() {
        let policy = TimeoutPolicy::default()
            .with_stale_timeout_for_template("heavy-review", Duration::from_secs(600))
            .with_stale_timeout_for_node_kind(NodeKindName::Session, Duration::from_secs(420));

        assert_eq!(
            policy.stale_timeout(&TimeoutContext::new(
                None,
                NodeKindName::Session,
                Some("heavy-review".to_string())
            )),
            Duration::from_secs(600)
        );
    }

    #[test]
    fn timeout_policy_prefers_node_kind_over_model() {
        let policy = TimeoutPolicy::default()
            .with_stale_timeout_for_node_kind(NodeKindName::Session, Duration::from_secs(420))
            .with_stale_timeout_for_model("slow-model", Duration::from_secs(480));

        assert_eq!(
            policy.stale_timeout(&TimeoutContext::new(
                Some("slow-model".to_string()),
                NodeKindName::Session,
                None
            )),
            Duration::from_secs(420)
        );
    }

    #[test]
    fn timeout_policy_resolves_approval_session_override() {
        let policy = TimeoutPolicy::default()
            .with_stale_timeout_for_approval_session(Duration::from_secs(420))
            .with_stale_timeout_for_model("slow-model", Duration::from_secs(480));

        assert_eq!(
            policy.stale_timeout(
                &TimeoutContext::new(Some("slow-model".to_string()), NodeKindName::Session, None)
                    .with_approval_gate(true)
            ),
            Duration::from_secs(420)
        );
        assert_eq!(
            policy.stale_timeout(&TimeoutContext::new(
                Some("slow-model".to_string()),
                NodeKindName::Session,
                None
            )),
            Duration::from_secs(480)
        );
    }

    #[test]
    fn timeout_policy_keeps_template_over_model_precedence() {
        let policy = TimeoutPolicy::default()
            .with_stale_timeout_for_template("heavy-review", Duration::from_secs(600))
            .with_stale_timeout_for_model("slow-model", Duration::from_secs(480));

        assert_eq!(
            policy.stale_timeout(&TimeoutContext::new(
                Some("slow-model".to_string()),
                NodeKindName::Session,
                Some("heavy-review".to_string())
            )),
            Duration::from_secs(600)
        );
    }
}
