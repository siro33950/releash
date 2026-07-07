use std::time::Duration;

use crate::domain::workflow::TimeoutPolicy;

const EXTENDED_STALE_TIMEOUT: Duration = Duration::from_secs(600);

const EXTENDED_STALE_MODELS: &[&str] = &["claude-opus-4-8", "gpt-5.6-sol"];

const EXTENDED_STALE_TEMPLATES: &[&str] = &[
    "02_implement_codex",
    "02_implement_claude",
    "03_review",
    "03_full-review",
    "04_review-fix-policy",
    "05_review-fix",
    "05_review-fix_codex",
    "05_review-fix_claude",
    "06_verify-review-comments",
];

pub(crate) fn workflow_runtime_timeout_policy() -> TimeoutPolicy {
    let policy = TimeoutPolicy::default();
    let policy = EXTENDED_STALE_MODELS.iter().fold(policy, |policy, model| {
        policy.with_stale_timeout_for_model(*model, EXTENDED_STALE_TIMEOUT)
    });
    let policy = policy.with_stale_timeout_for_approval_session(EXTENDED_STALE_TIMEOUT);
    EXTENDED_STALE_TEMPLATES
        .iter()
        .fold(policy, |policy, template| {
            policy.with_stale_timeout_for_template(*template, EXTENDED_STALE_TIMEOUT)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{NodeKindName, TimeoutContext};

    #[test]
    fn workflow_runtime_timeout_policy_injects_builtin_overrides_outside_domain() {
        let policy = workflow_runtime_timeout_policy();

        assert_eq!(
            policy.stale_timeout(&TimeoutContext::new(
                Some("gpt-5.5".to_string()),
                NodeKindName::Session,
                None
            )),
            EXTENDED_STALE_TIMEOUT
        );
        assert_eq!(
            policy.stale_timeout(&TimeoutContext::new(
                None,
                NodeKindName::Session,
                Some("05_review-fix_gpt55".to_string())
            )),
            EXTENDED_STALE_TIMEOUT
        );
        assert_eq!(
            policy.stale_timeout(&TimeoutContext::new(
                Some("unknown-fast".to_string()),
                NodeKindName::Session,
                Some("unknown-template".to_string())
            )),
            Duration::from_secs(180)
        );
        assert_eq!(
            policy.stale_timeout(
                &TimeoutContext::new(
                    Some("unknown-fast".to_string()),
                    NodeKindName::Session,
                    Some("unknown-template".to_string())
                )
                .with_approval_gate(true)
            ),
            EXTENDED_STALE_TIMEOUT
        );
        assert_eq!(
            TimeoutPolicy::default().stale_timeout(&TimeoutContext::new(
                Some("gpt-5.5".to_string()),
                NodeKindName::Session,
                Some("05_review-fix_gpt55".to_string())
            )),
            Duration::from_secs(180)
        );
    }
}
