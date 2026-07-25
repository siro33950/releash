use std::time::Duration;

use crate::domain::workflow::TimeoutPolicy;

const EXTENDED_STALE_TIMEOUT: Duration = Duration::from_secs(600);

const EXTENDED_STALE_MODELS: &[&str] = &["claude-opus-5", "gpt-5.6-sol"];

pub(crate) fn workflow_runtime_timeout_policy() -> TimeoutPolicy {
    let policy = TimeoutPolicy::default();
    let policy = EXTENDED_STALE_MODELS.iter().fold(policy, |policy, model| {
        policy.with_stale_timeout_for_model(*model, EXTENDED_STALE_TIMEOUT)
    });
    policy.with_stale_timeout_for_approval_session(EXTENDED_STALE_TIMEOUT)
}

#[cfg(test)]
mod tests {
    use super::*;
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
