use std::sync::{Arc, Mutex};

use super::{
    ProviderHookHealthUsecase, ProviderLifecycleIngressUsecase, ProviderLifecycleUsecase,
    ProviderSessionStartTransaction, ProviderWorkflowStopCommand, ProviderWorkflowStopTransaction,
};
use crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway;
use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionOrigin};
use crate::domain::agent_session::repository::{
    ProviderAgentSessionRepository, ProviderAgentSessionRepositoryError,
    VersionedProviderAgentSession,
};
use crate::domain::provider_lifecycle::{
    ProviderHookHealth, ProviderHookHealthRepository, ProviderHookHealthRepositoryError,
    ProviderKind, ProviderLifecycleEventRepository, ProviderLifecycleIngressResult,
    ProviderLifecycleRepositoryError, ProviderLifecycleScope, ProviderLifecycleSignal,
    ProviderLifecycleSlotId, ProviderLifecycleUnavailableObservation,
    ProviderLifecycleUnavailableReason, ScopedProviderLifecycleEvent, VersionedProviderHookHealth,
};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::agent_session::ProviderAgentSessionUsecase;

struct MemoryAgentSessions {
    stored: Mutex<VersionedProviderAgentSession>,
    fail_save: bool,
    save_observed: Option<Arc<tokio::sync::Notify>>,
}

#[async_trait::async_trait]
impl ProviderAgentSessionRepository for MemoryAgentSessions {
    async fn create(
        &self,
        _session: AgentSession,
        _caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError> {
        Err(ProviderAgentSessionRepositoryError::AlreadyExists)
    }

    async fn create_with_lifecycle_events(
        &self,
        _session: AgentSession,
        _lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        _caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError> {
        Err(ProviderAgentSessionRepositoryError::AlreadyExists)
    }

    async fn find(
        &self,
        session_id: &str,
    ) -> Result<Option<VersionedProviderAgentSession>, ProviderAgentSessionRepositoryError> {
        let stored = self.stored.lock().unwrap();
        Ok((stored.session().id() == session_id).then(|| stored.clone()))
    }

    async fn save(
        &self,
        session: VersionedProviderAgentSession,
        _caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError> {
        if let Some(save_observed) = &self.save_observed {
            save_observed.notify_one();
        }
        if self.fail_save {
            return Err(ProviderAgentSessionRepositoryError::Unavailable);
        }
        let previous_revision = session.revision();
        let mut entity = session.into_session();
        let event_count = entity.take_uncommitted_events().len() as u64;
        let saved =
            VersionedProviderAgentSession::restored(entity, previous_revision + event_count);
        *self.stored.lock().unwrap() = saved.clone();
        Ok(saved)
    }

    async fn remove(
        &self,
        _session: VersionedProviderAgentSession,
        _authorization: crate::domain::agent_session::aggregates::AgentSessionRemovalAuthorization,
        _caller_request_id: &str,
    ) -> Result<(), ProviderAgentSessionRepositoryError> {
        unreachable!()
    }
}

#[async_trait::async_trait]
impl ProviderSessionStartTransaction for MemoryAgentSessions {
    async fn commit_session_started(
        &self,
        session: VersionedProviderAgentSession,
        _lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError> {
        self.save(session, caller_request_id).await
    }
}

#[derive(Default)]
struct MemoryWorkflowStops {
    commits: Mutex<
        Vec<(
            ProviderWorkflowStopCommand,
            Vec<ScopedProviderLifecycleEvent>,
        )>,
    >,
}

#[async_trait::async_trait]
impl ProviderWorkflowStopTransaction for MemoryWorkflowStops {
    async fn commit_provider_stop(
        &self,
        command: ProviderWorkflowStopCommand,
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
struct MemoryLifecycleEvents;

#[async_trait::async_trait]
impl ProviderLifecycleEventRepository for MemoryLifecycleEvents {
    async fn append(
        &self,
        _events: Vec<ScopedProviderLifecycleEvent>,
    ) -> Result<(), ProviderLifecycleRepositoryError> {
        Ok(())
    }
}

#[derive(Default)]
struct MemoryHookHealth {
    stored: Mutex<std::collections::HashMap<ProviderKind, VersionedProviderHookHealth>>,
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

#[tokio::test]
async fn workflow_origin_stop_uses_the_atomic_provider_workflow_commit_boundary() {
    let mut session = AgentSession::create(
        "agent-workflow-stop",
        WorkspaceIdentity::new("/repo"),
        "/repo/worktree",
        ProviderKind::Codex,
        AgentSessionOrigin::workflow_node("workflow-1", "node-execution-1").unwrap(),
    )
    .unwrap();
    session.take_uncommitted_events();
    let agent_repository = Arc::new(MemoryAgentSessions {
        stored: Mutex::new(VersionedProviderAgentSession::restored(session, 1)),
        fail_save: false,
        save_observed: None,
    });
    let transaction = Arc::new(MemoryWorkflowStops::default());
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(MemoryLifecycleEvents),
    ));
    let ingress = ProviderLifecycleIngressUsecase::new(
        lifecycle.clone(),
        Arc::new(ProviderAgentSessionUsecase::new(agent_repository.clone())),
        Arc::new(ProviderHookHealthUsecase::new(Arc::new(
            MemoryHookHealth::default(),
        ))),
        agent_repository,
        transaction.clone(),
    );
    let slot_id = ProviderLifecycleSlotId::new("slot-workflow-stop").unwrap();
    let scope = ProviderLifecycleScope::new("agent-workflow-stop").unwrap();
    let armed = lifecycle
        .arm(slot_id.clone(), ProviderKind::Codex, scope.clone())
        .await
        .unwrap();
    ingress
        .receive(
            &slot_id,
            armed.capability(),
            ProviderLifecycleSignal::session_started(
                armed.binding_id(),
                ProviderKind::Codex,
                scope.clone(),
                "codex-session-1",
                None,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let result = ingress
        .receive(
            &slot_id,
            armed.capability(),
            ProviderLifecycleSignal::stop_observed(
                armed.binding_id(),
                ProviderKind::Codex,
                scope,
                "codex-session-1",
                None,
            )
            .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(result, ProviderLifecycleIngressResult::Applied);
    {
        let commits = transaction.commits.lock().unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].0.agent_session_id, "agent-workflow-stop");
        assert_eq!(commits[0].0.workflow_execution_id, "workflow-1");
        assert_eq!(commits[0].0.node_execution_id, "node-execution-1");
        assert_eq!(commits[0].0.binding_id, armed.binding_id());
        assert_eq!(commits[0].1.len(), 1);
    }

    let next_turn = ingress
        .receive(
            &slot_id,
            armed.capability(),
            ProviderLifecycleSignal::stop_observed(
                armed.binding_id(),
                ProviderKind::Codex,
                ProviderLifecycleScope::new("agent-workflow-stop").unwrap(),
                "codex-session-1",
                None,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(next_turn, ProviderLifecycleIngressResult::Applied);
    {
        let commits = transaction.commits.lock().unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[1].1.len(), 1);
    }

    let wrong_binding = ingress
        .receive(
            &slot_id,
            armed.capability(),
            ProviderLifecycleSignal::stop_observed(
                "different-binding",
                ProviderKind::Codex,
                ProviderLifecycleScope::new("agent-workflow-stop").unwrap(),
                "codex-session-1",
                None,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert!(matches!(
        wrong_binding,
        ProviderLifecycleIngressResult::Rejected(
            crate::domain::provider_lifecycle::ProviderLifecycleRejection::BindingExpired
        )
    ));
    assert_eq!(transaction.commits.lock().unwrap().len(), 2);

    let wrong_session = ingress
        .receive(
            &slot_id,
            armed.capability(),
            ProviderLifecycleSignal::stop_observed(
                armed.binding_id(),
                ProviderKind::Codex,
                ProviderLifecycleScope::new("different-agent-session").unwrap(),
                "codex-session-1",
                None,
            )
            .unwrap(),
        )
        .await;
    assert_eq!(
        wrong_session.unwrap_err(),
        super::ProviderLifecycleIngressUsecaseError::InvalidInput
    );
    assert_eq!(transaction.commits.lock().unwrap().len(), 2);

    let stop_failure = ingress
        .receive(
            &slot_id,
            armed.capability(),
            ProviderLifecycleSignal::stop_failed(
                armed.binding_id(),
                ProviderKind::Codex,
                ProviderLifecycleScope::new("agent-workflow-stop").unwrap(),
                "codex-session-1",
                None,
                "hook failed",
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stop_failure, ProviderLifecycleIngressResult::Applied);
    assert_eq!(transaction.commits.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn standalone_stop_does_not_enter_the_workflow_transaction() {
    let mut session = AgentSession::create(
        "agent-standalone-stop",
        WorkspaceIdentity::new("/repo"),
        "/repo/worktree",
        ProviderKind::Claude,
        AgentSessionOrigin::Standalone,
    )
    .unwrap();
    session.take_uncommitted_events();
    let agent_repository = Arc::new(MemoryAgentSessions {
        stored: Mutex::new(VersionedProviderAgentSession::restored(session, 1)),
        fail_save: false,
        save_observed: None,
    });
    let transaction = Arc::new(MemoryWorkflowStops::default());
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(MemoryLifecycleEvents),
    ));
    let ingress = ProviderLifecycleIngressUsecase::new(
        lifecycle.clone(),
        Arc::new(ProviderAgentSessionUsecase::new(agent_repository.clone())),
        Arc::new(ProviderHookHealthUsecase::new(Arc::new(
            MemoryHookHealth::default(),
        ))),
        agent_repository,
        transaction.clone(),
    );
    let slot_id = ProviderLifecycleSlotId::new("slot-standalone-stop").unwrap();
    let scope = ProviderLifecycleScope::new("agent-standalone-stop").unwrap();
    let armed = lifecycle
        .arm(slot_id.clone(), ProviderKind::Claude, scope.clone())
        .await
        .unwrap();
    ingress
        .receive(
            &slot_id,
            armed.capability(),
            ProviderLifecycleSignal::session_started(
                armed.binding_id(),
                ProviderKind::Claude,
                scope.clone(),
                "claude-session-1",
                None,
            )
            .unwrap(),
        )
        .await
        .unwrap();

    let result = ingress
        .receive(
            &slot_id,
            armed.capability(),
            ProviderLifecycleSignal::stop_observed(
                armed.binding_id(),
                ProviderKind::Claude,
                scope,
                "claude-session-1",
                None,
            )
            .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(result, ProviderLifecycleIngressResult::Applied);
    assert!(transaction.commits.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_provider_lifecycle_ingress_session_startでwarningを解除しsession_idを所有する() {
    let mut session = AgentSession::create(
        "agent-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/worktree",
        ProviderKind::Codex,
        AgentSessionOrigin::Standalone,
    )
    .unwrap();
    session.take_uncommitted_events();
    let agent_repository = Arc::new(MemoryAgentSessions {
        stored: Mutex::new(VersionedProviderAgentSession::restored(session, 1)),
        fail_save: false,
        save_observed: None,
    });
    let sessions = Arc::new(ProviderAgentSessionUsecase::new(agent_repository.clone()));
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(MemoryLifecycleEvents),
    ));
    let health = Arc::new(ProviderHookHealthUsecase::new(Arc::new(
        MemoryHookHealth::default(),
    )));
    let ingress = ProviderLifecycleIngressUsecase::new(
        lifecycle.clone(),
        sessions,
        health.clone(),
        agent_repository.clone(),
        Arc::new(MemoryWorkflowStops::default()),
    );
    let slot_id = ProviderLifecycleSlotId::new("slot-1").unwrap();
    let scope = ProviderLifecycleScope::new("agent-1").unwrap();
    let armed = lifecycle
        .arm(slot_id.clone(), ProviderKind::Codex, scope.clone())
        .await
        .unwrap();
    health
        .record_launch(
            ProviderKind::Codex,
            slot_id.as_str(),
            "launch-before-unavailable",
        )
        .await
        .unwrap();
    let unavailable = ProviderLifecycleUnavailableObservation::new(
        armed.binding_id(),
        ProviderKind::Codex,
        scope.clone(),
        ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed,
    )
    .unwrap();
    ingress
        .report_unavailable(&slot_id, armed.capability(), unavailable)
        .await
        .unwrap();
    assert_eq!(health.warnings().await.unwrap().len(), 1);

    let signal = ProviderLifecycleSignal::session_started(
        armed.binding_id(),
        ProviderKind::Codex,
        scope,
        "codex-session-1",
        Some("/provider/rollout.jsonl"),
    )
    .unwrap();
    ingress
        .receive(&slot_id, armed.capability(), signal)
        .await
        .unwrap();

    assert!(health.warnings().await.unwrap().is_empty());
    let stored = agent_repository.stored.lock().unwrap();
    assert_eq!(
        stored.session().provider_session_id(),
        Some("codex-session-1")
    );
    assert_eq!(
        stored.session().transcript_ref(),
        Some("/provider/rollout.jsonl")
    );
}

#[tokio::test]
async fn test_provider_lifecycle_ingress_session関連付け失敗時はwarningを解除しない() {
    let mut session = AgentSession::create(
        "agent-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/worktree",
        ProviderKind::Codex,
        AgentSessionOrigin::Standalone,
    )
    .unwrap();
    session.take_uncommitted_events();
    let agent_repository = Arc::new(MemoryAgentSessions {
        stored: Mutex::new(VersionedProviderAgentSession::restored(session, 1)),
        fail_save: true,
        save_observed: None,
    });
    let sessions = Arc::new(ProviderAgentSessionUsecase::new(agent_repository.clone()));
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(MemoryLifecycleEvents),
    ));
    let health = Arc::new(ProviderHookHealthUsecase::new(Arc::new(
        MemoryHookHealth::default(),
    )));
    let ingress = ProviderLifecycleIngressUsecase::new(
        lifecycle.clone(),
        sessions,
        health.clone(),
        agent_repository,
        Arc::new(MemoryWorkflowStops::default()),
    );
    let slot_id = ProviderLifecycleSlotId::new("slot-failed-association").unwrap();
    let scope = ProviderLifecycleScope::new("agent-1").unwrap();
    let armed = lifecycle
        .arm(slot_id.clone(), ProviderKind::Codex, scope.clone())
        .await
        .unwrap();
    health
        .record_launch(
            ProviderKind::Codex,
            slot_id.as_str(),
            "launch-before-warning",
        )
        .await
        .unwrap();
    health
        .record_unavailable(
            ProviderKind::Codex,
            slot_id.as_str(),
            ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed,
            "warning-before-session-start",
        )
        .await
        .unwrap();

    let error = ingress
        .receive(
            &slot_id,
            armed.capability(),
            ProviderLifecycleSignal::session_started(
                armed.binding_id(),
                ProviderKind::Codex,
                scope,
                "codex-session-1",
                None,
            )
            .unwrap(),
        )
        .await
        .unwrap_err();

    assert_eq!(
        error,
        super::ProviderLifecycleIngressUsecaseError::StorageUnavailable
    );
    assert_eq!(health.warnings().await.unwrap().len(), 1);
}

#[tokio::test]
async fn test_provider_lifecycle_ingress_session関連付け拒否時にlifecycleを確定しない() {
    let mut session = AgentSession::create(
        "agent-consistent",
        WorkspaceIdentity::new("/repo"),
        "/repo/worktree",
        ProviderKind::Codex,
        AgentSessionOrigin::Standalone,
    )
    .unwrap();
    session.take_uncommitted_events();
    session
        .associate_provider_session("codex-session-correct", None)
        .unwrap();
    session.take_uncommitted_events();
    let agent_repository = Arc::new(MemoryAgentSessions {
        stored: Mutex::new(VersionedProviderAgentSession::restored(session, 2)),
        fail_save: false,
        save_observed: None,
    });
    let sessions = Arc::new(ProviderAgentSessionUsecase::new(agent_repository.clone()));
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(MemoryLifecycleEvents),
    ));
    let ingress = ProviderLifecycleIngressUsecase::new(
        lifecycle.clone(),
        sessions,
        Arc::new(ProviderHookHealthUsecase::new(Arc::new(
            MemoryHookHealth::default(),
        ))),
        agent_repository,
        Arc::new(MemoryWorkflowStops::default()),
    );
    let slot_id = ProviderLifecycleSlotId::new("slot-consistent").unwrap();
    let scope = ProviderLifecycleScope::new("agent-consistent").unwrap();
    let armed = lifecycle
        .arm(slot_id.clone(), ProviderKind::Codex, scope.clone())
        .await
        .unwrap();

    let wrong = ingress
        .receive(
            &slot_id,
            armed.capability(),
            ProviderLifecycleSignal::session_started(
                armed.binding_id(),
                ProviderKind::Codex,
                scope.clone(),
                "codex-session-wrong",
                None,
            )
            .unwrap(),
        )
        .await;
    assert_eq!(
        wrong.unwrap_err(),
        super::ProviderLifecycleIngressUsecaseError::InvalidInput
    );

    let correct = ingress
        .receive(
            &slot_id,
            armed.capability(),
            ProviderLifecycleSignal::session_started(
                armed.binding_id(),
                ProviderKind::Codex,
                scope,
                "codex-session-correct",
                None,
            )
            .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(correct, ProviderLifecycleIngressResult::Applied);
}

#[tokio::test]
async fn test_provider_lifecycle_ingress_session操作lock解放後にsession_startを関連付ける() {
    let mut session = AgentSession::create(
        "agent-locked",
        WorkspaceIdentity::new("/repo"),
        "/repo/worktree",
        ProviderKind::Claude,
        AgentSessionOrigin::Standalone,
    )
    .unwrap();
    session.take_uncommitted_events();
    let save_observed = Arc::new(tokio::sync::Notify::new());
    let agent_repository = Arc::new(MemoryAgentSessions {
        stored: Mutex::new(VersionedProviderAgentSession::restored(session, 1)),
        fail_save: false,
        save_observed: Some(save_observed.clone()),
    });
    let sessions = Arc::new(ProviderAgentSessionUsecase::new(agent_repository.clone()));
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(MemoryLifecycleEvents),
    ));
    let health = Arc::new(ProviderHookHealthUsecase::new(Arc::new(
        MemoryHookHealth::default(),
    )));
    let ingress = Arc::new(ProviderLifecycleIngressUsecase::new(
        lifecycle.clone(),
        sessions.clone(),
        health,
        agent_repository,
        Arc::new(MemoryWorkflowStops::default()),
    ));
    let slot_id = ProviderLifecycleSlotId::new("slot-locked").unwrap();
    let scope = ProviderLifecycleScope::new("agent-locked").unwrap();
    let armed = lifecycle
        .arm(slot_id.clone(), ProviderKind::Claude, scope.clone())
        .await
        .unwrap();
    let operation = sessions.lock_operation("agent-locked").await.unwrap();
    let receive = tokio::spawn({
        let ingress = ingress.clone();
        let capability = armed.capability().to_string();
        let binding_id = armed.binding_id().to_string();
        async move {
            ingress
                .receive(
                    &slot_id,
                    &capability,
                    ProviderLifecycleSignal::session_started(
                        &binding_id,
                        ProviderKind::Claude,
                        scope,
                        "claude-session-locked",
                        None,
                    )
                    .unwrap(),
                )
                .await
        }
    });

    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            save_observed.notified(),
        )
        .await
        .is_err(),
        "同じAgentSessionの操作lock中にSessionStartを保存してはならない"
    );

    drop(operation);
    assert!(receive.await.unwrap().is_ok());
}
