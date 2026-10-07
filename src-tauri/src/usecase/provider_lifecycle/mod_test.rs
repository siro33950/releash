use crate::usecase::agent_session::test_helpers::session_location;
use crate::usecase::agent_session::test_helpers::workflow_location;

mod provider_lifecycle_ingress_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use super::super::{
        ProviderHookHealthUsecase, ProviderLifecycleIngressUsecase,
        ProviderLifecycleIngressUsecaseError, ProviderLifecycleUsecase,
    };
    use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionTreeLocation};
    use crate::domain::agent_session::repository::VersionedAgentSession;
    use crate::domain::provider_lifecycle::{
        ProviderKind, ProviderLifecycleIngressResult, ProviderLifecycleScope,
        ProviderLifecycleSignal, ProviderLifecycleSlotId, ProviderLifecycleUnavailableObservation,
        ProviderLifecycleUnavailableReason,
    };
    use crate::domain::workflow::AgentSessionActivity;
    use crate::domain::workspace_tree::WorkspaceIdentity;
    use crate::usecase::agent_session::AgentSessionUsecase;
    use crate::usecase::test_helpers::TestCredentials as LocalProviderLifecycleCredentialGateway;

    use super::super::test_helpers::*;

    #[tokio::test]
    async fn workflow_origin_stop_uses_the_atomic_provider_workflow_commit_boundary() {
        let mut session = AgentSession::create(
            "agent-workflow-stop",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            workflow_location("workflow-1", "node-execution-1"),
        )
        .unwrap();
        session.observe_activity(AgentSessionActivity::Working);
        session.take_uncommitted_events();
        let agent_repository = Arc::new(MemoryAgentSessions {
            stored: Mutex::new(VersionedAgentSession::restored(session, 1)),
            fail_save: false,
            fail_activity_save: false,
            save_observed: None,
        });
        let transaction = Arc::new(MemoryWorkflowStops::default());
        let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(LocalProviderLifecycleCredentialGateway),
            Arc::new(MemoryLifecycleEvents),
        ));
        let notifier = Arc::new(RecordingChangeNotifier::default());
        let ingress = ProviderLifecycleIngressUsecase::new(
            std::sync::Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderPayloadInterpreter,
            ),
            std::sync::Arc::new(crate::usecase::test_helpers::TestIdentity),
            lifecycle.clone(),
            Arc::new(AgentSessionUsecase::new(agent_repository.clone())),
            Arc::new(ProviderHookHealthUsecase::new(Arc::new(
                MemoryHookHealth::default(),
            ))),
            (agent_repository.clone(), transaction.clone()),
            notifier.subscriptions.clone(),
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
        let revision_before_stop = agent_repository.stored.lock().unwrap().revision();
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
        assert_eq!(
            agent_repository.stored.lock().unwrap().revision(),
            revision_before_stop
        );
        assert_eq!(
            agent_repository.stored.lock().unwrap().session().activity(),
            AgentSessionActivity::Working
        );
        assert!(notifier.worktree_paths.lock().unwrap().is_empty());
        {
            let commits = transaction.commits.lock().unwrap();
            assert_eq!(commits.len(), 1);
            assert_eq!(commits[0].0.agent_session_id, "agent-workflow-stop");
            assert_eq!(commits[0].0.tree_id, "workflow-1");
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
            super::super::ProviderLifecycleIngressUsecaseError::InvalidInput
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
    async fn test_worktree削除中_provider_hookの状態変更を保存前に拒否する() {
        // Given
        let id = "agent-deleting-hook";
        let mut session = AgentSession::create(
            id,
            WorkspaceIdentity::new("/repo/worktree"),
            "/repo/worktree",
            ProviderKind::Codex,
            session_location(id),
        )
        .unwrap();
        session.take_uncommitted_events();
        let repository = Arc::new(MemoryAgentSessions {
            stored: Mutex::new(VersionedAgentSession::restored(session, 1)),
            fail_save: false,
            fail_activity_save: false,
            save_observed: None,
        });
        let before = repository.stored.lock().unwrap().clone();
        let transaction = Arc::new(MemoryWorkflowStops::default());
        let health = Arc::new(MemoryHookHealth::default());
        let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(LocalProviderLifecycleCredentialGateway),
            Arc::new(MemoryLifecycleEvents),
        ));
        let ingress = ProviderLifecycleIngressUsecase::new(
            std::sync::Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderPayloadInterpreter,
            ),
            std::sync::Arc::new(crate::usecase::test_helpers::TestIdentity),
            lifecycle.clone(),
            Arc::new(AgentSessionUsecase::new(repository.clone())),
            Arc::new(ProviderHookHealthUsecase::new(health.clone())),
            (repository.clone(), transaction.clone()),
            crate::test_support::state_subscription::test_subscriptions(),
        );
        let slot = ProviderLifecycleSlotId::new("slot-deleting-hook").unwrap();
        let scope = ProviderLifecycleScope::new(id).unwrap();
        let armed = lifecycle
            .arm(slot.clone(), ProviderKind::Codex, scope.clone())
            .await
            .unwrap();
        let _deletion = transaction
            .operations
            .delete("/repo/worktree")
            .await
            .unwrap();
        // When / Then
        for signal in [
            ProviderLifecycleSignal::session_started(
                armed.binding_id(),
                ProviderKind::Codex,
                scope.clone(),
                "provider",
                None,
            )
            .unwrap(),
            ProviderLifecycleSignal::stop_observed(
                armed.binding_id(),
                ProviderKind::Codex,
                scope.clone(),
                "provider",
                None,
            )
            .unwrap(),
            ProviderLifecycleSignal::activity_observed(
                armed.binding_id(),
                ProviderKind::Codex,
                scope.clone(),
                "provider",
                None,
                AgentSessionActivity::Working,
            )
            .unwrap(),
        ] {
            assert_eq!(
                ingress.receive(&slot, armed.capability(), signal).await,
                Err(ProviderLifecycleIngressUsecaseError::Conflict)
            );
        }
        let unavailable = ProviderLifecycleUnavailableObservation::new(
            armed.binding_id(),
            ProviderKind::Codex,
            scope,
            ProviderLifecycleUnavailableReason::LocalApiUnavailable,
        )
        .unwrap();
        assert_eq!(
            ingress
                .report_unavailable(&slot, armed.capability(), unavailable)
                .await,
            Err(ProviderLifecycleIngressUsecaseError::Conflict)
        );
        assert_eq!(*repository.stored.lock().unwrap(), before);
        assert!(transaction.commits.lock().unwrap().is_empty());
        assert!(health.stored.lock().unwrap().is_empty());
    }

    async fn assert_activity_ingress_for_location(
        agent_session_id: &str,
        location: AgentSessionTreeLocation,
    ) {
        let mut session = AgentSession::create(
            agent_session_id,
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            location,
        )
        .unwrap();
        session.take_uncommitted_events();
        let agent_repository = Arc::new(MemoryAgentSessions {
            stored: Mutex::new(VersionedAgentSession::restored(session, 1)),
            fail_save: false,
            fail_activity_save: false,
            save_observed: None,
        });
        let transaction = Arc::new(MemoryWorkflowStops::default());
        let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(LocalProviderLifecycleCredentialGateway),
            Arc::new(MemoryLifecycleEvents),
        ));
        let notifier = Arc::new(RecordingChangeNotifier::default());
        let ingress = ProviderLifecycleIngressUsecase::new(
            std::sync::Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderPayloadInterpreter,
            ),
            std::sync::Arc::new(crate::usecase::test_helpers::TestIdentity),
            lifecycle.clone(),
            Arc::new(AgentSessionUsecase::new(agent_repository.clone())),
            Arc::new(ProviderHookHealthUsecase::new(Arc::new(
                MemoryHookHealth::default(),
            ))),
            (agent_repository.clone(), transaction.clone()),
            notifier.subscriptions.clone(),
        );
        let slot_id = ProviderLifecycleSlotId::new(format!("slot-{agent_session_id}")).unwrap();
        let scope = ProviderLifecycleScope::new(agent_session_id).unwrap();
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
                    format!("provider-{agent_session_id}"),
                    None,
                )
                .unwrap(),
            )
            .await
            .unwrap();

        let working = || {
            ProviderLifecycleSignal::activity_observed(
                armed.binding_id(),
                ProviderKind::Codex,
                scope.clone(),
                format!("provider-{agent_session_id}"),
                None,
                AgentSessionActivity::Working,
            )
            .unwrap()
        };
        assert_eq!(
            ingress
                .receive(&slot_id, armed.capability(), working())
                .await
                .unwrap(),
            ProviderLifecycleIngressResult::Applied
        );
        let revision_after_transition = agent_repository.stored.lock().unwrap().revision();
        assert_eq!(
            ingress
                .receive(&slot_id, armed.capability(), working())
                .await
                .unwrap(),
            ProviderLifecycleIngressResult::Duplicate
        );
        assert_eq!(
            agent_repository.stored.lock().unwrap().revision(),
            revision_after_transition
        );
        assert_eq!(
            agent_repository.stored.lock().unwrap().session().activity(),
            AgentSessionActivity::Working
        );
        assert!(transaction.commits.lock().unwrap().is_empty());
        assert_eq!(
            notifier.worktree_paths.lock().unwrap().as_slice(),
            ["/repo"]
        );
    }

    #[tokio::test]
    async fn test_provider_lifecycle_ingress_活動観測はrootとworkflow子で同じ経路を使い同値を保存しない(
    ) {
        assert_activity_ingress_for_location(
            "agent-activity-root",
            session_location("agent-activity-root"),
        )
        .await;
        assert_activity_ingress_for_location(
            "agent-activity-child",
            workflow_location("workflow-activity", "node-activity"),
        )
        .await;
    }

    #[tokio::test]
    async fn test_provider_lifecycle_ingress_活動保存失敗は状態とrevisionと通知を変えない() {
        let mut session = AgentSession::create(
            "agent-activity-save-failure",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            session_location("agent-activity-save-failure"),
        )
        .unwrap();
        session.take_uncommitted_events();
        let agent_repository = Arc::new(MemoryAgentSessions {
            stored: Mutex::new(VersionedAgentSession::restored(session, 1)),
            fail_save: false,
            fail_activity_save: true,
            save_observed: None,
        });
        let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(LocalProviderLifecycleCredentialGateway),
            Arc::new(MemoryLifecycleEvents),
        ));
        let notifier = Arc::new(RecordingChangeNotifier::default());
        let ingress = ProviderLifecycleIngressUsecase::new(
            std::sync::Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderPayloadInterpreter,
            ),
            std::sync::Arc::new(crate::usecase::test_helpers::TestIdentity),
            lifecycle.clone(),
            Arc::new(AgentSessionUsecase::new(agent_repository.clone())),
            Arc::new(ProviderHookHealthUsecase::new(Arc::new(
                MemoryHookHealth::default(),
            ))),
            (
                agent_repository.clone(),
                Arc::new(MemoryWorkflowStops::default()),
            ),
            notifier.subscriptions.clone(),
        );
        let slot_id = ProviderLifecycleSlotId::new("slot-activity-save-failure").unwrap();
        let scope = ProviderLifecycleScope::new("agent-activity-save-failure").unwrap();
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
                    "codex-session-activity-save-failure",
                    None,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let before = agent_repository.stored.lock().unwrap().clone();
        let notifications_before = notifier.worktree_paths.lock().unwrap().clone();

        let error = ingress
            .receive(
                &slot_id,
                armed.capability(),
                ProviderLifecycleSignal::activity_observed(
                    armed.binding_id(),
                    ProviderKind::Codex,
                    scope,
                    "codex-session-activity-save-failure",
                    None,
                    AgentSessionActivity::Working,
                )
                .unwrap(),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error,
            ProviderLifecycleIngressUsecaseError::StorageUnavailable
        );
        let after = agent_repository.stored.lock().unwrap();
        assert_eq!(after.session().activity(), before.session().activity());
        assert_eq!(after.revision(), before.revision());
        assert_eq!(
            notifier.worktree_paths.lock().unwrap().as_slice(),
            notifications_before
        );
    }

    #[tokio::test]
    async fn test_provider_lifecycle_ingress_claudeのstop_failureは活動だけをawaiting_instructionへ戻す(
    ) {
        let mut session = AgentSession::create(
            "agent-stop-failure",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            workflow_location("workflow-stop-failure", "node-stop-failure"),
        )
        .unwrap();
        session.observe_activity(AgentSessionActivity::Working);
        session.take_uncommitted_events();
        let agent_repository = Arc::new(MemoryAgentSessions {
            stored: Mutex::new(VersionedAgentSession::restored(session, 1)),
            fail_save: false,
            fail_activity_save: false,
            save_observed: None,
        });
        let transaction = Arc::new(MemoryWorkflowStops::default());
        let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(LocalProviderLifecycleCredentialGateway),
            Arc::new(MemoryLifecycleEvents),
        ));
        let notifier = Arc::new(RecordingChangeNotifier::default());
        let ingress = ProviderLifecycleIngressUsecase::new(
            std::sync::Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderPayloadInterpreter,
            ),
            std::sync::Arc::new(crate::usecase::test_helpers::TestIdentity),
            lifecycle.clone(),
            Arc::new(AgentSessionUsecase::new(agent_repository.clone())),
            Arc::new(ProviderHookHealthUsecase::new(Arc::new(
                MemoryHookHealth::default(),
            ))),
            (agent_repository.clone(), transaction.clone()),
            notifier.subscriptions.clone(),
        );
        let slot_id = ProviderLifecycleSlotId::new("slot-stop-failure").unwrap();
        let scope = ProviderLifecycleScope::new("agent-stop-failure").unwrap();
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
                    "claude-session-stop-failure",
                    None,
                )
                .unwrap(),
            )
            .await
            .unwrap();

        let stop_failure = || {
            ProviderLifecycleSignal::stop_failed(
                armed.binding_id(),
                ProviderKind::Claude,
                scope.clone(),
                "claude-session-stop-failure",
                None,
                "provider request failed",
            )
            .unwrap()
        };
        let revision_before = agent_repository.stored.lock().unwrap().revision();
        assert_eq!(
            ingress
                .receive(&slot_id, armed.capability(), stop_failure())
                .await
                .unwrap(),
            ProviderLifecycleIngressResult::Applied
        );
        assert_eq!(
            agent_repository.stored.lock().unwrap().session().activity(),
            AgentSessionActivity::AwaitingInstruction
        );
        assert_eq!(
            agent_repository.stored.lock().unwrap().revision(),
            revision_before + 1
        );
        assert!(transaction.commits.lock().unwrap().is_empty());
        assert_eq!(
            notifier.worktree_paths.lock().unwrap().as_slice(),
            ["/repo"]
        );

        let revision_after_transition = agent_repository.stored.lock().unwrap().revision();
        ingress
            .receive(&slot_id, armed.capability(), stop_failure())
            .await
            .unwrap();
        assert_eq!(
            agent_repository.stored.lock().unwrap().revision(),
            revision_after_transition
        );
        assert!(transaction.commits.lock().unwrap().is_empty());
        assert_eq!(
            notifier.worktree_paths.lock().unwrap().as_slice(),
            ["/repo"]
        );
    }

    #[tokio::test]
    async fn standalone_stop_uses_the_same_execution_tree_transaction() {
        let mut session = AgentSession::create(
            "agent-standalone-stop",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            session_location("agent-standalone-stop"),
        )
        .unwrap();
        session.take_uncommitted_events();
        let agent_repository = Arc::new(MemoryAgentSessions {
            stored: Mutex::new(VersionedAgentSession::restored(session, 1)),
            fail_save: false,
            fail_activity_save: false,
            save_observed: None,
        });
        let transaction = Arc::new(MemoryWorkflowStops::default());
        let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(LocalProviderLifecycleCredentialGateway),
            Arc::new(MemoryLifecycleEvents),
        ));
        let ingress = ProviderLifecycleIngressUsecase::new(
            std::sync::Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderPayloadInterpreter,
            ),
            std::sync::Arc::new(crate::usecase::test_helpers::TestIdentity),
            lifecycle.clone(),
            Arc::new(AgentSessionUsecase::new(agent_repository.clone())),
            Arc::new(ProviderHookHealthUsecase::new(Arc::new(
                MemoryHookHealth::default(),
            ))),
            (agent_repository, transaction.clone()),
            crate::test_support::state_subscription::test_subscriptions(),
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
        let commits = transaction.commits.lock().unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].0.agent_session_id, "agent-standalone-stop");
        assert_eq!(commits[0].0.tree_id, "agent-standalone-stop");
        assert_eq!(commits[0].0.node_execution_id, "agent-standalone-stop");
    }

    #[tokio::test]
    async fn test_provider_lifecycle_ingress_session_startでwarningを解除しsession_idを所有する() {
        let mut session = AgentSession::create(
            "agent-1",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            session_location("agent-1"),
        )
        .unwrap();
        session.take_uncommitted_events();
        let agent_repository = Arc::new(MemoryAgentSessions {
            stored: Mutex::new(VersionedAgentSession::restored(session, 1)),
            fail_save: false,
            fail_activity_save: false,
            save_observed: None,
        });
        let sessions = Arc::new(AgentSessionUsecase::new(agent_repository.clone()));
        let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(LocalProviderLifecycleCredentialGateway),
            Arc::new(MemoryLifecycleEvents),
        ));
        let health = Arc::new(ProviderHookHealthUsecase::new(Arc::new(
            MemoryHookHealth::default(),
        )));
        let ingress = ProviderLifecycleIngressUsecase::new(
            std::sync::Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderPayloadInterpreter,
            ),
            std::sync::Arc::new(crate::usecase::test_helpers::TestIdentity),
            lifecycle.clone(),
            sessions,
            health.clone(),
            (
                agent_repository.clone(),
                Arc::new(MemoryWorkflowStops::default()),
            ),
            crate::test_support::state_subscription::test_subscriptions(),
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
            session_location("agent-1"),
        )
        .unwrap();
        session.take_uncommitted_events();
        let agent_repository = Arc::new(MemoryAgentSessions {
            stored: Mutex::new(VersionedAgentSession::restored(session, 1)),
            fail_save: true,
            fail_activity_save: false,
            save_observed: None,
        });
        let sessions = Arc::new(AgentSessionUsecase::new(agent_repository.clone()));
        let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(LocalProviderLifecycleCredentialGateway),
            Arc::new(MemoryLifecycleEvents),
        ));
        let health = Arc::new(ProviderHookHealthUsecase::new(Arc::new(
            MemoryHookHealth::default(),
        )));
        let ingress = ProviderLifecycleIngressUsecase::new(
            std::sync::Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderPayloadInterpreter,
            ),
            std::sync::Arc::new(crate::usecase::test_helpers::TestIdentity),
            lifecycle.clone(),
            sessions,
            health.clone(),
            (agent_repository, Arc::new(MemoryWorkflowStops::default())),
            crate::test_support::state_subscription::test_subscriptions(),
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
            super::super::ProviderLifecycleIngressUsecaseError::StorageUnavailable
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
            session_location("agent-consistent"),
        )
        .unwrap();
        session.take_uncommitted_events();
        session
            .associate_provider_session("codex-session-correct", None)
            .unwrap();
        session.take_uncommitted_events();
        let agent_repository = Arc::new(MemoryAgentSessions {
            stored: Mutex::new(VersionedAgentSession::restored(session, 2)),
            fail_save: false,
            fail_activity_save: false,
            save_observed: None,
        });
        let sessions = Arc::new(AgentSessionUsecase::new(agent_repository.clone()));
        let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(LocalProviderLifecycleCredentialGateway),
            Arc::new(MemoryLifecycleEvents),
        ));
        let ingress = ProviderLifecycleIngressUsecase::new(
            std::sync::Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderPayloadInterpreter,
            ),
            std::sync::Arc::new(crate::usecase::test_helpers::TestIdentity),
            lifecycle.clone(),
            sessions,
            Arc::new(ProviderHookHealthUsecase::new(Arc::new(
                MemoryHookHealth::default(),
            ))),
            (agent_repository, Arc::new(MemoryWorkflowStops::default())),
            crate::test_support::state_subscription::test_subscriptions(),
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
            super::super::ProviderLifecycleIngressUsecaseError::InvalidInput
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
            session_location("agent-locked"),
        )
        .unwrap();
        session.take_uncommitted_events();
        let save_observed = Arc::new(tokio::sync::Notify::new());
        let agent_repository = Arc::new(MemoryAgentSessions {
            stored: Mutex::new(VersionedAgentSession::restored(session, 1)),
            fail_save: false,
            fail_activity_save: false,
            save_observed: Some(save_observed.clone()),
        });
        let sessions = Arc::new(AgentSessionUsecase::new(agent_repository.clone()));
        let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(LocalProviderLifecycleCredentialGateway),
            Arc::new(MemoryLifecycleEvents),
        ));
        let health = Arc::new(ProviderHookHealthUsecase::new(Arc::new(
            MemoryHookHealth::default(),
        )));
        let ingress = Arc::new(ProviderLifecycleIngressUsecase::new(
            std::sync::Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderPayloadInterpreter,
            ),
            std::sync::Arc::new(crate::usecase::test_helpers::TestIdentity),
            lifecycle.clone(),
            sessions.clone(),
            health,
            (agent_repository, Arc::new(MemoryWorkflowStops::default())),
            crate::test_support::state_subscription::test_subscriptions(),
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

    #[tokio::test]
    async fn test_provider_lifecycle_ingress_session操作lock解放後に活動観測を保存する() {
        // Given: provider session を関連付け済みで、保存通知を観測できる AgentSession
        let mut session = AgentSession::create(
            "agent-activity-locked",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            session_location("agent-activity-locked"),
        )
        .unwrap();
        session.take_uncommitted_events();
        let save_observed = Arc::new(tokio::sync::Notify::new());
        let agent_repository = Arc::new(MemoryAgentSessions {
            stored: Mutex::new(VersionedAgentSession::restored(session, 1)),
            fail_save: false,
            fail_activity_save: false,
            save_observed: Some(save_observed.clone()),
        });
        let sessions = Arc::new(AgentSessionUsecase::new(agent_repository.clone()));
        let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(LocalProviderLifecycleCredentialGateway),
            Arc::new(MemoryLifecycleEvents),
        ));
        let ingress = Arc::new(ProviderLifecycleIngressUsecase::new(
            std::sync::Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderPayloadInterpreter,
            ),
            std::sync::Arc::new(crate::usecase::test_helpers::TestIdentity),
            lifecycle.clone(),
            sessions.clone(),
            Arc::new(ProviderHookHealthUsecase::new(Arc::new(
                MemoryHookHealth::default(),
            ))),
            (
                agent_repository.clone(),
                Arc::new(MemoryWorkflowStops::default()),
            ),
            crate::test_support::state_subscription::test_subscriptions(),
        ));
        let slot_id = ProviderLifecycleSlotId::new("slot-activity-locked").unwrap();
        let scope = ProviderLifecycleScope::new("agent-activity-locked").unwrap();
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
                    "claude-session-activity-locked",
                    None,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        save_observed.notified().await;

        // When: operation lock を外部で保持したまま活動 signal を受信する
        let operation = sessions
            .lock_operation("agent-activity-locked")
            .await
            .unwrap();
        let receive = tokio::spawn({
            let ingress = ingress.clone();
            let capability = armed.capability().to_string();
            let binding_id = armed.binding_id().to_string();
            async move {
                ingress
                    .receive(
                        &slot_id,
                        &capability,
                        ProviderLifecycleSignal::activity_observed(
                            &binding_id,
                            ProviderKind::Claude,
                            scope,
                            "claude-session-activity-locked",
                            None,
                            AgentSessionActivity::Working,
                        )
                        .unwrap(),
                    )
                    .await
            }
        });

        // Then: lock 保持中は保存されず、解放後に Applied となる
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(100),
                save_observed.notified(),
            )
            .await
            .is_err(),
            "同じAgentSessionの操作lock中に活動状態を保存してはならない"
        );
        drop(operation);
        assert_eq!(
            receive.await.unwrap().unwrap(),
            ProviderLifecycleIngressResult::Applied
        );
        assert_eq!(
            agent_repository.stored.lock().unwrap().session().activity(),
            AgentSessionActivity::Working
        );
    }
}

mod provider_lifecycle_usecase_tests {
    use std::sync::atomic::{AtomicU8, Ordering};

    use tokio::sync::Notify;

    use super::super::*;
    use crate::domain::provider_lifecycle::{
        IssuedProviderLifecycleCredential, ProviderHookHealth, ProviderHookHealthRepository,
        ProviderHookHealthRepositoryError, ProviderLifecycleCapabilityHash,
        ProviderLifecycleUnavailableReason, VersionedProviderHookHealth,
    };

    #[derive(Default)]
    struct FakeCredentials {
        next: AtomicU8,
    }

    impl ProviderLifecycleCredentialGateway for FakeCredentials {
        fn issue(&self) -> IssuedProviderLifecycleCredential {
            let sequence = self.next.fetch_add(1, Ordering::SeqCst) + 1;
            IssuedProviderLifecycleCredential::new(
                format!("binding-{sequence}"),
                format!("capability-{sequence}"),
                ProviderLifecycleCapabilityHash::from_digest([sequence; 32]),
            )
        }

        fn hash(&self, capability: &str) -> ProviderLifecycleCapabilityHash {
            let sequence = capability
                .strip_prefix("capability-")
                .and_then(|value| value.parse::<u8>().ok())
                .unwrap_or(0);
            ProviderLifecycleCapabilityHash::from_digest([sequence; 32])
        }
    }

    #[derive(Default)]
    struct RecordingEvents {
        batches: Mutex<Vec<Vec<ScopedProviderLifecycleEvent>>>,
        failure: Mutex<Option<ProviderLifecycleRepositoryError>>,
    }

    impl RecordingEvents {
        fn fail_with(&self, error: ProviderLifecycleRepositoryError) {
            *self.failure.lock().unwrap() = Some(error);
        }

        fn clear_failure(&self) {
            *self.failure.lock().unwrap() = None;
        }

        fn batch_count(&self) -> usize {
            self.batches.lock().unwrap().len()
        }
    }

    #[async_trait::async_trait]
    impl ProviderLifecycleEventRepository for RecordingEvents {
        async fn append(
            &self,
            events: Vec<ScopedProviderLifecycleEvent>,
        ) -> Result<(), ProviderLifecycleRepositoryError> {
            if let Some(error) = self.failure.lock().unwrap().clone() {
                return Err(error);
            }
            self.batches.lock().unwrap().push(events);
            Ok(())
        }

        async fn load_scope(
            &self,
            scope: &ProviderLifecycleScope,
        ) -> Result<Vec<ScopedProviderLifecycleEvent>, ProviderLifecycleRepositoryError> {
            Ok(self
                .batches
                .lock()
                .unwrap()
                .iter()
                .flatten()
                .filter_map(|event| {
                    let (event_scope, event) = event.clone().into_parts();
                    (event_scope == *scope)
                        .then(|| ScopedProviderLifecycleEvent::new(event_scope, event))
                })
                .collect())
        }
    }

    struct ConcurrentEvents {
        blocked_entered: Notify,
        independent_entered: Notify,
        release_blocked: Notify,
    }

    impl ConcurrentEvents {
        fn new() -> Self {
            Self {
                blocked_entered: Notify::new(),
                independent_entered: Notify::new(),
                release_blocked: Notify::new(),
            }
        }
    }

    #[async_trait::async_trait]
    impl ProviderLifecycleEventRepository for ConcurrentEvents {
        async fn append(
            &self,
            events: Vec<ScopedProviderLifecycleEvent>,
        ) -> Result<(), ProviderLifecycleRepositoryError> {
            let agent_session_id = events
                .into_iter()
                .next()
                .map(ScopedProviderLifecycleEvent::into_parts)
                .map(|(scope, _)| scope.agent_session_id().to_string())
                .unwrap();
            match agent_session_id.as_str() {
                "agent-blocked" => {
                    self.blocked_entered.notify_one();
                    self.release_blocked.notified().await;
                }
                "agent-independent" => self.independent_entered.notify_one(),
                _ => {}
            }
            Ok(())
        }

        async fn load_scope(
            &self,
            _scope: &ProviderLifecycleScope,
        ) -> Result<Vec<ScopedProviderLifecycleEvent>, ProviderLifecycleRepositoryError> {
            Ok(Vec::new())
        }
    }

    fn slot_id(value: &str) -> ProviderLifecycleSlotId {
        ProviderLifecycleSlotId::new(value).unwrap()
    }

    fn scope(session: &str) -> ProviderLifecycleScope {
        ProviderLifecycleScope::new(session).unwrap()
    }

    fn session_start(
        armed: &ArmedProviderLifecycle,
        provider_session_id: &str,
    ) -> ProviderLifecycleSignal {
        ProviderLifecycleSignal::session_started(
            armed.binding_id(),
            armed.provider(),
            armed.scope().clone(),
            provider_session_id,
            None,
        )
        .unwrap()
    }

    fn usecase() -> (
        ProviderLifecycleUsecase,
        Arc<RecordingEvents>,
        Arc<FakeCredentials>,
    ) {
        let events = Arc::new(RecordingEvents::default());
        let credentials = Arc::new(FakeCredentials::default());
        (
            ProviderLifecycleUsecase::new(credentials.clone(), events.clone()),
            events,
            credentials,
        )
    }

    #[tokio::test]
    async fn test_providerライフサイクルusecase_永続化失敗時は直前bindingをcurrentに保つ() {
        let (usecase, events, _) = usecase();
        let slot = slot_id("slot-1");
        let previous = usecase
            .arm(slot.clone(), ProviderKind::Codex, scope("agent-previous"))
            .await
            .unwrap();
        events.fail_with(ProviderLifecycleRepositoryError::StorageUnavailable);

        assert_eq!(
            usecase
                .arm(slot.clone(), ProviderKind::Codex, scope("agent-current"),)
                .await,
            Err(ProviderLifecycleUsecaseError::StorageUnavailable)
        );
        events.clear_failure();

        assert_eq!(
            usecase
                .receive(
                    &slot,
                    previous.capability(),
                    session_start(&previous, "provider-session-previous"),
                )
                .await
                .unwrap(),
            ProviderLifecycleIngressResult::Applied
        );
        assert_eq!(events.batch_count(), 2);
    }

    #[tokio::test]
    async fn test_providerライフサイクルusecase_同一slotの再起動が旧bindingを拒否する() {
        let (usecase, events, _) = usecase();
        let slot = slot_id("slot-1");
        let previous = usecase
            .arm(slot.clone(), ProviderKind::Codex, scope("agent-previous"))
            .await
            .unwrap();
        usecase
            .arm(slot.clone(), ProviderKind::Codex, scope("agent-current"))
            .await
            .unwrap();

        assert_eq!(
            usecase
                .receive(
                    &slot,
                    previous.capability(),
                    session_start(&previous, "provider-session-previous"),
                )
                .await
                .unwrap(),
            ProviderLifecycleIngressResult::Rejected(ProviderLifecycleRejection::BindingExpired)
        );
        assert_eq!(events.batch_count(), 2);
    }

    #[tokio::test]
    async fn test_providerライフサイクルusecase_異なるslotは互いのbindingを失効させない() {
        let (usecase, _, _) = usecase();
        let first_slot = slot_id("slot-1");
        let second_slot = slot_id("slot-2");
        let first = usecase
            .arm(first_slot.clone(), ProviderKind::Claude, scope("agent-1"))
            .await
            .unwrap();
        usecase
            .arm(second_slot, ProviderKind::Claude, scope("agent-2"))
            .await
            .unwrap();

        assert_eq!(
            usecase
                .receive(
                    &first_slot,
                    first.capability(),
                    session_start(&first, "provider-session-1"),
                )
                .await
                .unwrap(),
            ProviderLifecycleIngressResult::Applied
        );
    }

    #[tokio::test]
    async fn test_providerライフサイクルusecase_一方のslot永続化中も別slotを処理する() {
        let events = Arc::new(ConcurrentEvents::new());
        let usecase = Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(FakeCredentials::default()),
            events.clone(),
        ));
        let blocked = {
            let usecase = usecase.clone();
            tokio::spawn(async move {
                usecase
                    .arm(
                        slot_id("slot-blocked"),
                        ProviderKind::Claude,
                        scope("agent-blocked"),
                    )
                    .await
            })
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            events.blocked_entered.notified(),
        )
        .await
        .unwrap();

        let independent = {
            let usecase = usecase.clone();
            tokio::spawn(async move {
                usecase
                    .arm(
                        slot_id("slot-independent"),
                        ProviderKind::Codex,
                        scope("agent-independent"),
                    )
                    .await
            })
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            events.independent_entered.notified(),
        )
        .await
        .unwrap();
        assert!(independent.await.unwrap().is_ok());

        events.release_blocked.notify_one();
        assert!(blocked.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn test_providerライフサイクルusecase_明示releaseでlive_slotを除去する() {
        let (usecase, _, _) = usecase();
        let slot = slot_id("slot-1");
        let armed = usecase
            .arm(slot.clone(), ProviderKind::Claude, scope("agent-1"))
            .await
            .unwrap();
        assert_eq!(usecase.live_slot_count().unwrap(), 1);

        assert_eq!(
            usecase.release(&slot, armed.binding_id()).await.unwrap(),
            ProviderLifecycleIngressResult::Applied
        );
        assert_eq!(usecase.live_slot_count().unwrap(), 0);
        assert_eq!(
            usecase
                .receive(
                    &slot,
                    armed.capability(),
                    session_start(&armed, "provider-session-1"),
                )
                .await
                .unwrap(),
            ProviderLifecycleIngressResult::Rejected(ProviderLifecycleRejection::BindingNotActive)
        );
    }

    #[tokio::test]
    async fn test_providerライフサイクルusecase_agent_session終了でscopeのbindingを全て失効する() {
        let (usecase, _, _) = usecase();
        let target_scope = scope("agent-1");
        let first = usecase
            .arm(
                slot_id("slot-1"),
                ProviderKind::Claude,
                target_scope.clone(),
            )
            .await
            .unwrap();
        let second = usecase
            .arm(
                slot_id("slot-2"),
                ProviderKind::Claude,
                target_scope.clone(),
            )
            .await
            .unwrap();
        usecase
            .arm(
                slot_id("slot-other"),
                ProviderKind::Codex,
                scope("agent-other"),
            )
            .await
            .unwrap();

        assert_eq!(usecase.release_scope(&target_scope).await.unwrap(), 2);
        assert_eq!(usecase.live_slot_count().unwrap(), 1);
        for armed in [first, second] {
            assert_eq!(
                usecase
                    .receive(
                        armed.slot_id(),
                        armed.capability(),
                        session_start(&armed, "provider-session-1"),
                    )
                    .await
                    .unwrap(),
                ProviderLifecycleIngressResult::Rejected(
                    ProviderLifecycleRejection::BindingNotActive
                )
            );
        }
    }

    #[tokio::test]
    async fn test_providerライフサイクルusecase_再起動後もscopeのbindingを失効する() {
        let events = Arc::new(RecordingEvents::default());
        let credentials = Arc::new(FakeCredentials::default());
        let before_restart = ProviderLifecycleUsecase::new(credentials.clone(), events.clone());
        let target_scope = scope("agent-restarted");
        before_restart
            .arm(
                slot_id("slot-before-restart"),
                ProviderKind::Codex,
                target_scope.clone(),
            )
            .await
            .unwrap();

        let after_restart = ProviderLifecycleUsecase::new(credentials, events.clone());
        assert_eq!(after_restart.release_scope(&target_scope).await.unwrap(), 1);

        let persisted = events.load_scope(&target_scope).await.unwrap();
        assert!(matches!(
            persisted.last().cloned().map(|event| event.into_parts().1),
            Some(ProviderLifecycleEvent::BindingExpired { .. })
        ));
        assert_eq!(after_restart.release_scope(&target_scope).await.unwrap(), 0);
    }

    #[test]
    fn test_providerライフサイクルusecase_repository_errorをusecase_errorへ変換する() {
        for (repository_error, expected) in [
            (
                ProviderLifecycleRepositoryError::InvalidInput,
                ProviderLifecycleUsecaseError::InvalidInput,
            ),
            (
                ProviderLifecycleRepositoryError::StorageUnavailable,
                ProviderLifecycleUsecaseError::StorageUnavailable,
            ),
            (
                ProviderLifecycleRepositoryError::Corrupt,
                ProviderLifecycleUsecaseError::Corrupt,
            ),
        ] {
            assert_eq!(
                ProviderLifecycleUsecaseError::from(repository_error),
                expected
            );
        }
    }

    #[derive(Default)]
    struct InMemoryHookHealthRepository {
        stored: Mutex<std::collections::HashMap<ProviderKind, VersionedProviderHookHealth>>,
    }

    #[async_trait::async_trait]
    impl ProviderHookHealthRepository for InMemoryHookHealthRepository {
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
            let event_count = health.health_mut().take_uncommitted_events().len() as u64;
            let revision = health.revision() + event_count;
            let saved = VersionedProviderHookHealth::restored(health.into_health(), revision);
            self.stored
                .lock()
                .unwrap()
                .insert(saved.health().provider(), saved.clone());
            Ok(saved)
        }
    }

    #[tokio::test]
    async fn test_provider_hook_health_usecase_異常を警告し後続session_startで解除する() {
        let repository = Arc::new(InMemoryHookHealthRepository::default());
        let usecase = ProviderHookHealthUsecase::new(repository);

        usecase
            .record_launch(ProviderKind::Codex, "launch-1", "launch-request-1")
            .await
            .unwrap();
        usecase
            .record_unavailable(
                ProviderKind::Codex,
                "launch-1",
                ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed,
                "warning-request-1",
            )
            .await
            .unwrap();
        assert_eq!(
            usecase.warnings().await.unwrap(),
            vec![ProviderHookHealthWarning {
                provider: ProviderKind::Codex,
                launch_id: "launch-1".to_string(),
                reason: ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed,
            }]
        );

        usecase
            .record_launch(ProviderKind::Codex, "launch-2", "launch-request-2")
            .await
            .unwrap();
        usecase
            .record_session_started(ProviderKind::Codex, "launch-2", "clear-request-1")
            .await
            .unwrap();
        assert!(usecase.warnings().await.unwrap().is_empty());
    }

    struct FixedHookDeliveryFailures {
        observations:
            Vec<Result<ProviderHookHealthFailureObservation, ProviderHookHealthFailureQueryError>>,
    }

    #[async_trait::async_trait]
    impl ProviderHookHealthFailureQuery for FixedHookDeliveryFailures {
        async fn list(
            &self,
            _limit: usize,
        ) -> Result<
            Vec<Result<ProviderHookHealthFailureObservation, ProviderHookHealthFailureQueryError>>,
            ProviderHookHealthFailureQueryError,
        > {
            Ok(self.observations.clone())
        }
    }

    #[tokio::test]
    async fn test_provider_hook_health_read_local_api配送失敗を最新launchの警告へ反映する() {
        // Given
        let repository = Arc::new(InMemoryHookHealthRepository::default());
        let health = Arc::new(ProviderHookHealthUsecase::new(repository));
        health
            .record_launch(
                ProviderKind::Claude,
                "launch-latest",
                "launch-latest-request",
            )
            .await
            .unwrap();
        let read = ProviderHookHealthReadUsecase::new(
            health,
            Arc::new(FixedHookDeliveryFailures {
                observations: vec![
                    Ok(ProviderHookHealthFailureObservation {
                        provider: ProviderKind::Claude,
                        launch_id: "launch-old".to_string(),
                        reason: ProviderLifecycleUnavailableReason::LocalApiUnavailable,
                    }),
                    Ok(ProviderHookHealthFailureObservation {
                        provider: ProviderKind::Claude,
                        launch_id: "launch-latest".to_string(),
                        reason: ProviderLifecycleUnavailableReason::LocalApiUnavailable,
                    }),
                ],
            }),
        );

        // When
        let result = read.warnings().await.unwrap();
        // Then
        assert_eq!(
            result.warnings,
            vec![ProviderHookHealthWarning {
                provider: ProviderKind::Claude,
                launch_id: "launch-latest".to_string(),
                reason: ProviderLifecycleUnavailableReason::LocalApiUnavailable,
            }]
        );
    }

    #[tokio::test]
    async fn test_provider警告_正常session_start後の同一launch欠落報告を無視する() {
        // Given
        let repository = Arc::new(InMemoryHookHealthRepository::default());
        let health = ProviderHookHealthUsecase::new(repository);
        health
            .record_launch(ProviderKind::Claude, "launch-1", "launch-1-request")
            .await
            .unwrap();

        // When
        health
            .record_unavailable(
                ProviderKind::Claude,
                "launch-1",
                ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded,
                "missing-1-request",
            )
            .await
            .unwrap();
        let before = health.warnings().await.unwrap();

        health
            .record_session_started(ProviderKind::Claude, "launch-1", "session-started-request")
            .await
            .unwrap();
        health
            .record_unavailable(
                ProviderKind::Claude,
                "launch-1",
                ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded,
                "missing-after-success-request",
            )
            .await
            .unwrap();
        let after = health.warnings().await.unwrap();
        // Then
        assert_eq!(before.len(), 1);
        assert!(after.is_empty());
    }

    #[tokio::test]
    async fn test_provider警告読取_正常な警告と記録ごとの失敗を一緒に返す() {
        // Given
        let health = Arc::new(ProviderHookHealthUsecase::new(Arc::new(
            InMemoryHookHealthRepository::default(),
        )));
        health
            .record_launch(ProviderKind::Claude, "launch", "launch-request")
            .await
            .unwrap();
        let read = ProviderHookHealthReadUsecase::new(
            health,
            Arc::new(FixedHookDeliveryFailures {
                observations: vec![
                    Err(ProviderHookHealthFailureQueryError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Transient,
                            message: "unavailable".into(),
                        },
                    )),
                    Ok(ProviderHookHealthFailureObservation {
                        provider: ProviderKind::Claude,
                        launch_id: "launch".into(),
                        reason: ProviderLifecycleUnavailableReason::LocalApiUnavailable,
                    }),
                    Err(ProviderHookHealthFailureQueryError::Corrupt),
                ],
            }),
        );
        // When
        let result = read.warnings().await.unwrap();
        // Then
        assert_eq!(
            result.warnings,
            vec![ProviderHookHealthWarning {
                provider: ProviderKind::Claude,
                launch_id: "launch".into(),
                reason: ProviderLifecycleUnavailableReason::LocalApiUnavailable
            }]
        );
        assert_eq!(
            result.failures,
            vec![
                ProviderHookHealthFailureQueryError::Technical(
                    crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Transient,
                        message: "unavailable".into()
                    }
                ),
                ProviderHookHealthFailureQueryError::Corrupt
            ]
        );
    }
}
