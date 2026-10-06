pub(crate) mod tests {
    use super::super::*;
    use crate::domain::workflow::{NodeKindName, TimeoutContext};

    fn timeout_for(model: &str) -> Duration {
        workflow_runtime_timeout_policy().stale_timeout(&TimeoutContext::new(
            Some(model.to_string()),
            NodeKindName::Session,
            None,
        ))
    }

    #[test]
    fn opus_5_uses_extended_stale_timeout() {
        assert_eq!(timeout_for("claude-opus-5"), EXTENDED_STALE_TIMEOUT);
    }

    #[test]
    fn removed_opus_model_does_not_keep_extended_stale_timeout() {
        assert_eq!(timeout_for("claude-opus-4-8"), Duration::from_secs(180));
    }
}
