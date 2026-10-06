#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::time::Duration;

#[cfg(test)]
use crate::domain::workflow::value_objects::NodeKindName;

#[cfg(test)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TimeoutContext {
    pub model: Option<String>,
    pub node_kind: NodeKindName,
    pub approval_gate: bool,
    pub workflow_template: Option<String>,
}

#[cfg(test)]
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

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeoutPolicy {
    startup_timeout: Duration,
    stale_timeout: Duration,
    stale_timeout_by_model: HashMap<String, Duration>,
    stale_timeout_by_node_kind: HashMap<NodeKindName, Duration>,
    stale_timeout_for_approval_session: Option<Duration>,
    stale_timeout_by_template: HashMap<String, Duration>,
}

#[cfg(test)]
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

    #[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
#[path = "failure_policy_test.rs"]
mod failure_policy_tests;
