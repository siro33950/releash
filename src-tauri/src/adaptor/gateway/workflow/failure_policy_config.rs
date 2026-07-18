use std::time::Duration;

use crate::domain::workflow::TimeoutPolicy;

const EXTENDED_STALE_TIMEOUT: Duration = Duration::from_secs(600);

const EXTENDED_STALE_MODELS: &[&str] = &["claude-opus-4-8", "gpt-5.6-sol"];

const EXTENDED_STALE_TEMPLATES: &[&str] = &["full-cycle-development", "06_verify-review-comments"];

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
