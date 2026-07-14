use std::collections::HashMap;
use std::time::Duration;

use crate::domain::workflow::value_objects::{NodeExecutionFailureKind, NodeKindName};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryPolicy {
    max_retries_by_kind: HashMap<NodeExecutionFailureKind, u32>,
}

impl RetryPolicy {
    pub fn should_retry(&self, kind: NodeExecutionFailureKind, attempts: u32) -> bool {
        attempts < self.max_retries(kind)
    }

    pub fn max_retries(&self, kind: NodeExecutionFailureKind) -> u32 {
        self.max_retries_by_kind.get(&kind).copied().unwrap_or(0)
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries_by_kind: HashMap::from([
                (NodeExecutionFailureKind::StartupTimeout, 2),
                (NodeExecutionFailureKind::StaleRuntimeTimeout, 0),
            ]),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TimeoutContext {
    pub model: Option<String>,
    pub node_kind: NodeKindName,
    pub approval_gate: bool,
    pub workflow_template: Option<String>,
}

impl TimeoutContext {
    pub fn new(
        model: Option<String>,
        node_kind: NodeKindName,
        workflow_template: Option<String>,
    ) -> Self {
        Self {
            model,
            node_kind,
            approval_gate: false,
            workflow_template,
        }
    }

    pub fn with_approval_gate(mut self, approval_gate: bool) -> Self {
        self.approval_gate = approval_gate;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeoutPolicy {
    startup_timeout: Duration,
    stale_timeout: Duration,
    stale_timeout_by_model: HashMap<String, Duration>,
    stale_timeout_by_node_kind: HashMap<NodeKindName, Duration>,
    stale_timeout_for_approval_session: Option<Duration>,
    stale_timeout_by_template: HashMap<String, Duration>,
}

impl TimeoutPolicy {
    pub fn startup_timeout(&self, _ctx: &TimeoutContext) -> Duration {
        self.startup_timeout
    }

    pub fn with_stale_timeout_for_model(
        mut self,
        model: impl Into<String>,
        timeout: Duration,
    ) -> Self {
        self.stale_timeout_by_model.insert(model.into(), timeout);
        self
    }

    pub fn with_stale_timeout_for_template(
        mut self,
        template: impl Into<String>,
        timeout: Duration,
    ) -> Self {
        self.stale_timeout_by_template
            .insert(template.into(), timeout);
        self
    }

    #[cfg(test)]
    pub fn with_stale_timeout_for_node_kind(
        mut self,
        node_kind: NodeKindName,
        timeout: Duration,
    ) -> Self {
        self.stale_timeout_by_node_kind.insert(node_kind, timeout);
        self
    }

    pub fn with_stale_timeout_for_approval_session(mut self, timeout: Duration) -> Self {
        self.stale_timeout_for_approval_session = Some(timeout);
        self
    }

    pub fn stale_timeout(&self, ctx: &TimeoutContext) -> Duration {
        if let Some(template) = ctx.workflow_template.as_deref() {
            if let Some(timeout) = self.stale_timeout_by_template.get(template) {
                return *timeout;
            }
        }
        if ctx.approval_gate {
            if let Some(timeout) = self.stale_timeout_for_approval_session {
                return timeout;
            }
        }
        if let Some(timeout) = self.stale_timeout_by_node_kind.get(&ctx.node_kind) {
            return *timeout;
        }
        if let Some(model) = ctx.model.as_deref() {
            if let Some(timeout) = self.stale_timeout_by_model.get(model) {
                return *timeout;
            }
        }
        self.stale_timeout
    }
}

impl Default for TimeoutPolicy {
    fn default() -> Self {
        Self {
            startup_timeout: Duration::from_secs(30),
            stale_timeout: Duration::from_secs(180),
            stale_timeout_by_model: HashMap::new(),
            stale_timeout_by_node_kind: HashMap::new(),
            stale_timeout_for_approval_session: None,
            stale_timeout_by_template: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairDecision {
    Repair { attempt: u32 },
    GiveUp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredOutputRepairPolicy {
    max_attempts: u32,
}

impl StructuredOutputRepairPolicy {
    pub fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    pub fn decide(&self, prior_attempts: u32, has_session: bool) -> RepairDecision {
        if !has_session || prior_attempts >= self.max_attempts {
            RepairDecision::GiveUp
        } else {
            RepairDecision::Repair {
                attempt: prior_attempts + 1,
            }
        }
    }
}

impl Default for StructuredOutputRepairPolicy {
    fn default() -> Self {
        Self { max_attempts: 2 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_policy_retries_startup_timeout_twice_only() {
        let policy = RetryPolicy::default();

        assert!(policy.should_retry(NodeExecutionFailureKind::StartupTimeout, 0));
        assert!(policy.should_retry(NodeExecutionFailureKind::StartupTimeout, 1));
        assert!(!policy.should_retry(NodeExecutionFailureKind::StartupTimeout, 2));
    }

    #[test]
    fn retry_policy_keeps_stale_retry_disabled_by_default() {
        let policy = RetryPolicy::default();

        assert_eq!(
            policy.max_retries(NodeExecutionFailureKind::StaleRuntimeTimeout),
            0
        );
        assert!(!policy.should_retry(NodeExecutionFailureKind::StaleRuntimeTimeout, 0));
    }

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

    #[test]
    fn artifact_repair_policy_limits_attempts_and_requires_session() {
        let policy = StructuredOutputRepairPolicy::default();

        assert_eq!(
            policy.decide(0, true),
            RepairDecision::Repair { attempt: 1 }
        );
        assert_eq!(
            policy.decide(1, true),
            RepairDecision::Repair { attempt: 2 }
        );
        assert_eq!(policy.decide(2, true), RepairDecision::GiveUp);
        assert_eq!(policy.decide(0, false), RepairDecision::GiveUp);
    }
}
