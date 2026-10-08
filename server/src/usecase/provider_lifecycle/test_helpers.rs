use std::sync::{Arc, Mutex};

use super::{
    ProviderExecutionTreeStopCommand, ProviderExecutionTreeStopTransaction,
    ProviderSessionStartTransaction,
};
use crate::domain::agent_session::aggregates::AgentSession;
use crate::domain::agent_session::repository::{
    AgentSessionRepository, AgentSessionRepositoryError, VersionedAgentSession,
};
use crate::domain::provider_lifecycle::{
    ProviderHookHealth, ProviderHookHealthRepository, ProviderHookHealthRepositoryError,
    ProviderKind, ProviderLifecycleEventRepository, ProviderLifecycleRepositoryError,
    ProviderLifecycleScope, ScopedProviderLifecycleEvent, VersionedProviderHookHealth,
};
pub(super) struct RecordingChangeNotifier {
    pub(super) subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
    pub(super) worktree_paths: crate::test_support::state_subscription::CapturingNotifier<String>,
}
impl Default for RecordingChangeNotifier {
    fn default() -> Self {
        let subscriptions = crate::test_support::state_subscription::test_subscriptions();
        let worktree_paths =
            crate::test_support::state_subscription::CapturingNotifier::worktrees(&subscriptions);
        Self {
            subscriptions,
            worktree_paths,
        }
    }
}

pub(super) struct MemoryAgentSessions {
    pub(super) stored: Mutex<VersionedAgentSession>,
    pub(super) fail_save: bool,
    pub(super) fail_activity_save: bool,
    pub(super) save_observed: Option<Arc<tokio::sync::Notify>>,
}

#[async_trait::async_trait]
impl AgentSessionRepository for MemoryAgentSessions {
    async fn create(
        &self,
        _session: AgentSession,
        _caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        Err(AgentSessionRepositoryError::Conflict)
    }

    async fn create_with_lifecycle_events(
        &self,
        _session: AgentSession,
        _lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        _caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        Err(AgentSessionRepositoryError::Conflict)
    }

    async fn find(
        &self,
        session_id: &str,
    ) -> Result<Option<VersionedAgentSession>, AgentSessionRepositoryError> {
        let stored = self.stored.lock().unwrap();
        Ok((stored.session().id() == session_id).then(|| stored.clone()))
    }

    async fn save(
        &self,
        session: VersionedAgentSession,
        _caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        if let Some(save_observed) = &self.save_observed {
            save_observed.notify_one();
        }
        if self.fail_save {
            return Err(AgentSessionRepositoryError::Unavailable);
        }
        let previous_revision = session.revision();
        let mut entity = session.into_session();
        let event_count = entity.take_uncommitted_events().len() as u64;
        let saved = VersionedAgentSession::restored(entity, previous_revision + event_count);
        *self.stored.lock().unwrap() = saved.clone();
        Ok(saved)
    }

    async fn save_activity(
        &self,
        session: VersionedAgentSession,
        caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        if self.fail_activity_save {
            return Err(AgentSessionRepositoryError::Unavailable);
        }
        self.save(session, caller_request_id).await
    }

    async fn remove(
        &self,
        _session: VersionedAgentSession,
        _authorization: crate::domain::agent_session::aggregates::AgentSessionRemovalAuthorization,
        _caller_request_id: &str,
    ) -> Result<(), AgentSessionRepositoryError> {
        unreachable!()
    }
}

#[async_trait::async_trait]
impl ProviderSessionStartTransaction for MemoryAgentSessions {
    async fn commit_session_started(
        &self,
        session: VersionedAgentSession,
        _lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        self.save(session, caller_request_id).await
    }
}

#[derive(Default)]
pub(super) struct MemoryWorkflowStops {
    pub(super) operations: crate::usecase::worktree_operation::WorktreeOperations,
    pub(super) commits: Mutex<
        Vec<(
            ProviderExecutionTreeStopCommand,
            Vec<ScopedProviderLifecycleEvent>,
        )>,
    >,
}

#[async_trait::async_trait]
impl ProviderExecutionTreeStopTransaction for MemoryWorkflowStops {
    fn begin_worktree_mutation(
        &self,
        path: &str,
    ) -> Result<
        crate::usecase::worktree_operation::WorktreeMutationGuard,
        super::ProviderLifecycleIngressUsecaseError,
    > {
        self.operations
            .mutate(path)
            .map_err(|_| super::ProviderLifecycleIngressUsecaseError::Conflict)
    }

    async fn commit_provider_stop(
        &self,
        command: ProviderExecutionTreeStopCommand,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
    ) -> Result<(), super::ProviderLifecycleIngressUsecaseError> {
        self.commits
            .lock()
            .unwrap()
            .push((command, lifecycle_events));
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct MemoryLifecycleEvents;

#[async_trait::async_trait]
impl ProviderLifecycleEventRepository for MemoryLifecycleEvents {
    async fn append(
        &self,
        _events: Vec<ScopedProviderLifecycleEvent>,
    ) -> Result<(), ProviderLifecycleRepositoryError> {
        Ok(())
    }

    async fn load_scope(
        &self,
        _scope: &ProviderLifecycleScope,
    ) -> Result<Vec<ScopedProviderLifecycleEvent>, ProviderLifecycleRepositoryError> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
pub(super) struct MemoryHookHealth {
    pub(super) stored: Mutex<std::collections::HashMap<ProviderKind, VersionedProviderHookHealth>>,
}

#[async_trait::async_trait]
impl ProviderHookHealthRepository for MemoryHookHealth {
    async fn load(
        &self,
        provider: ProviderKind,
    ) -> Result<VersionedProviderHookHealth, ProviderHookHealthRepositoryError> {
        Ok(self
            .stored
            .lock()
            .unwrap()
            .get(&provider)
            .cloned()
            .unwrap_or_else(|| {
                VersionedProviderHookHealth::restored(ProviderHookHealth::new(provider), 0)
            }))
    }

    async fn save(
        &self,
        mut health: VersionedProviderHookHealth,
        _caller_request_id: &str,
    ) -> Result<VersionedProviderHookHealth, ProviderHookHealthRepositoryError> {
        let revision =
            health.revision() + health.health_mut().take_uncommitted_events().len() as u64;
        let saved = VersionedProviderHookHealth::restored(health.into_health(), revision);
        self.stored
            .lock()
            .unwrap()
            .insert(saved.health().provider(), saved.clone());
        Ok(saved)
    }
}
