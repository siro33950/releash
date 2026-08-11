use crate::domain::provider_lifecycle::{ProviderKind, ProviderLifecycleUnavailableReason};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderHookHealthEvent {
    LaunchObserved {
        provider: ProviderKind,
        launch_id: String,
    },
    WarningRecorded {
        provider: ProviderKind,
        launch_id: String,
        reason: ProviderLifecycleUnavailableReason,
    },
    SessionStartedObserved {
        provider: ProviderKind,
        launch_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderHookHealthOutcome {
    Applied(ProviderHookHealthEvent),
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderHookWarning {
    launch_id: String,
    reason: ProviderLifecycleUnavailableReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderHookHealth {
    provider: ProviderKind,
    latest_launch_id: Option<String>,
    session_started_launch_id: Option<String>,
    warning: Option<ProviderHookWarning>,
    uncommitted_events: Vec<ProviderHookHealthEvent>,
}

impl ProviderHookHealth {
    pub(crate) fn new(provider: ProviderKind) -> Self {
        Self {
            provider,
            latest_launch_id: None,
            session_started_launch_id: None,
            warning: None,
            uncommitted_events: Vec::new(),
        }
    }

    pub(crate) fn rehydrate(
        provider: ProviderKind,
        events: &[ProviderHookHealthEvent],
    ) -> Option<Self> {
        let mut health = Self::new(provider);
        for event in events {
            match event {
                ProviderHookHealthEvent::LaunchObserved {
                    provider: event_provider,
                    launch_id,
                } if *event_provider == provider && !launch_id.trim().is_empty() => {
                    health.latest_launch_id = Some(launch_id.clone());
                    health.session_started_launch_id = None;
                }
                ProviderHookHealthEvent::WarningRecorded {
                    provider: event_provider,
                    launch_id,
                    reason,
                } if *event_provider == provider && !launch_id.trim().is_empty() => {
                    health.warning = Some(ProviderHookWarning {
                        launch_id: launch_id.clone(),
                        reason: *reason,
                    });
                }
                ProviderHookHealthEvent::SessionStartedObserved {
                    provider: event_provider,
                    launch_id,
                } if *event_provider == provider
                    && health.latest_launch_id.as_deref() == Some(launch_id)
                    && !launch_id.trim().is_empty() =>
                {
                    health.session_started_launch_id = Some(launch_id.clone());
                    health.warning = None;
                }
                _ => return None,
            }
        }
        health.uncommitted_events.clear();
        Some(health)
    }

    pub(crate) fn provider(&self) -> ProviderKind {
        self.provider
    }

    pub(crate) fn warning(&self) -> Option<(&str, ProviderLifecycleUnavailableReason)> {
        self.warning
            .as_ref()
            .map(|warning| (warning.launch_id.as_str(), warning.reason))
    }

    pub(crate) fn latest_launch_id(&self) -> Option<&str> {
        self.latest_launch_id.as_deref()
    }

    pub(crate) fn latest_launch_session_started(&self) -> bool {
        self.latest_launch_id.is_some()
            && self.latest_launch_id.as_deref() == self.session_started_launch_id.as_deref()
    }

    pub(crate) fn take_uncommitted_events(&mut self) -> Vec<ProviderHookHealthEvent> {
        std::mem::take(&mut self.uncommitted_events)
    }

    pub(crate) fn observe_unavailable(
        &mut self,
        launch_id: &str,
        reason: ProviderLifecycleUnavailableReason,
    ) -> ProviderHookHealthOutcome {
        if self.latest_launch_id() != Some(launch_id)
            || (self.session_started_launch_id.as_deref() == Some(launch_id)
                && matches!(
                    reason,
                    ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded
                        | ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed
                ))
        {
            return ProviderHookHealthOutcome::Duplicate;
        }
        if self.warning() == Some((launch_id, reason)) {
            return ProviderHookHealthOutcome::Duplicate;
        }
        self.warning = Some(ProviderHookWarning {
            launch_id: launch_id.to_string(),
            reason,
        });
        let event = ProviderHookHealthEvent::WarningRecorded {
            provider: self.provider,
            launch_id: launch_id.to_string(),
            reason,
        };
        self.uncommitted_events.push(event.clone());
        ProviderHookHealthOutcome::Applied(event)
    }

    pub(crate) fn observe_session_started(&mut self, launch_id: &str) -> ProviderHookHealthOutcome {
        if self.latest_launch_id() != Some(launch_id)
            || (self.session_started_launch_id.as_deref() == Some(launch_id)
                && self.warning.is_none())
        {
            return ProviderHookHealthOutcome::Duplicate;
        }
        self.session_started_launch_id = Some(launch_id.to_string());
        self.warning = None;
        let event = ProviderHookHealthEvent::SessionStartedObserved {
            provider: self.provider,
            launch_id: launch_id.to_string(),
        };
        self.uncommitted_events.push(event.clone());
        ProviderHookHealthOutcome::Applied(event)
    }

    pub(crate) fn observe_active_session_started(
        &mut self,
        launch_id: &str,
    ) -> ProviderHookHealthOutcome {
        if self.latest_launch_id() != Some(launch_id) {
            self.observe_launch(launch_id);
        }
        self.observe_session_started(launch_id)
    }

    pub(crate) fn observe_launch(&mut self, launch_id: &str) -> ProviderHookHealthOutcome {
        if self.latest_launch_id() == Some(launch_id) {
            return ProviderHookHealthOutcome::Duplicate;
        }
        self.latest_launch_id = Some(launch_id.to_string());
        self.session_started_launch_id = None;
        let event = ProviderHookHealthEvent::LaunchObserved {
            provider: self.provider,
            launch_id: launch_id.to_string(),
        };
        self.uncommitted_events.push(event.clone());
        ProviderHookHealthOutcome::Applied(event)
    }
}
