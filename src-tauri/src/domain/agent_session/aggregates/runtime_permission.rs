use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionWaitDiagnostic {
    pub request_id: String,
    pub elapsed: Duration,
}

#[derive(Debug, Clone)]
struct PermissionVisibility {
    request_id: String,
    last_seen_at: Instant,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimePermission {
    pending_request_id: Option<String>,
    wait_started_at: Option<Instant>,
    diagnostic_emitted: bool,
    visibility: Option<PermissionVisibility>,
    revision: u64,
}

impl RuntimePermission {
    pub fn begin_wait(&mut self, request_id: String, started_at: Instant) -> u64 {
        self.pending_request_id = Some(request_id);
        self.wait_started_at = Some(started_at);
        self.diagnostic_emitted = false;
        self.visibility = None;
        self.bump_revision()
    }

    pub fn clear(&mut self) -> u64 {
        self.pending_request_id = None;
        self.wait_started_at = None;
        self.diagnostic_emitted = false;
        self.visibility = None;
        self.bump_revision()
    }

    pub fn resolve(&mut self, now: Instant) -> (u64, Option<Duration>) {
        let elapsed = self
            .wait_started_at
            .take()
            .map(|started_at| now.saturating_duration_since(started_at));
        self.pending_request_id = None;
        self.diagnostic_emitted = false;
        self.visibility = None;
        (self.bump_revision(), elapsed)
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn owns_pending_request(&self, request_id: &str) -> bool {
        self.pending_request_id.as_deref() == Some(request_id)
    }

    pub fn report_visibility(&mut self, request_id: &str, visible: bool, at: Instant) {
        if visible {
            if self.pending_request_id.as_deref() == Some(request_id) {
                self.visibility = Some(PermissionVisibility {
                    request_id: request_id.to_string(),
                    last_seen_at: at,
                });
            }
            return;
        }
        if self
            .visibility
            .as_ref()
            .is_some_and(|visibility| visibility.request_id == request_id)
        {
            self.visibility = None;
        }
    }

    pub fn mark_diagnostic_if_due(
        &mut self,
        now: Instant,
        threshold: Duration,
        observed_ttl: Duration,
    ) -> Option<PermissionWaitDiagnostic> {
        let request_id = self.pending_request_id.as_ref()?;
        if self.diagnostic_emitted {
            return None;
        }
        let started_at = self.wait_started_at?;
        let elapsed = now.saturating_duration_since(started_at);
        if elapsed < threshold {
            return None;
        }
        let observed = self.visibility.as_ref().is_some_and(|visibility| {
            visibility.request_id == *request_id
                && now.saturating_duration_since(visibility.last_seen_at) <= observed_ttl
        });
        if observed {
            return None;
        }
        self.diagnostic_emitted = true;
        Some(PermissionWaitDiagnostic {
            request_id: request_id.clone(),
            elapsed,
        })
    }

    #[cfg(test)]
    pub fn diagnostic_emitted(&self) -> bool {
        self.diagnostic_emitted
    }

    #[cfg(test)]
    pub fn visible_request_id(&self) -> Option<&str> {
        self.visibility
            .as_ref()
            .map(|visibility| visibility.request_id.as_str())
    }

    fn bump_revision(&mut self) -> u64 {
        self.revision = self.revision.saturating_add(1);
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_requires_an_unobserved_request_past_the_threshold() {
        let now = Instant::now();
        let mut state = RuntimePermission::default();
        state.begin_wait("request".into(), now - Duration::from_secs(2));
        state.report_visibility("request", true, now);
        assert!(state
            .mark_diagnostic_if_due(now, Duration::from_secs(1), Duration::from_secs(1))
            .is_none());
        state.report_visibility("request", false, now);
        assert_eq!(
            state
                .mark_diagnostic_if_due(now, Duration::from_secs(1), Duration::from_secs(1))
                .map(|diagnostic| diagnostic.request_id),
            Some("request".into())
        );
        assert!(state
            .mark_diagnostic_if_due(now, Duration::from_secs(1), Duration::from_secs(1))
            .is_none());
    }

    #[test]
    fn resolve_is_one_transition_for_revision_and_measurement() {
        let now = Instant::now();
        let mut state = RuntimePermission::default();
        assert_eq!(
            state.begin_wait("request".into(), now - Duration::from_secs(2)),
            1
        );
        let (revision, elapsed) = state.resolve(now);
        assert_eq!(revision, 2);
        assert_eq!(elapsed, Some(Duration::from_secs(2)));
    }
}
