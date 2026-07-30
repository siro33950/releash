mod tests {
    use super::*;
    use crate::adaptor::gateway::agent_session::session_storage::{
        AgentSessionProjectionCodecV1, FileSessionStorage,
    };
    use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
    use crate::domain::agent_session::gateway::{AgentBackend, AgentSessionRuntime};
    use crate::domain::agent_session::value_objects::{
        BackendCapabilities, ModelDescriptor, SkillEntry,
    };
    use crate::domain::local_event::{
        CommitIdentity, CommitOperationKind, IdempotencyBinding, LocalAtomicBatch,
        LocalEventTransactionRepository, LocalStateMutation, ObligationMutation, PendingIndexEntry,
        PendingPartition, Revision, RevisionGuard,
    };
    use crate::domain::workflow::WorkflowNodeContext;
    use crate::test_support::{
        build_agent_runtime_usecase_with_controller,
        build_agent_runtime_usecase_with_controller_and_notifiers,
        build_agent_runtime_usecase_with_controller_and_spawner,
        build_agent_runtime_usecase_with_controller_and_workspace_query, build_session_store,
        TestRuntimeCallKind,
    };
    use crate::usecase::agent_session::runtime::ports::{
        AgentSessionEventNotifier, AgentSessionStateChangedPayload, AgentStallObservedPayload,
        AgentStreamingDeltaPayload, WorkflowStallNotifier,
    };
    use crate::usecase::agent_session::session::{
        create_session_internal_with_attributes, ChatMessage, MessagePart, PermissionPartStatus,
        PermissionRequestKindMsg, PermissionRequestMsg, SessionCreationAttributes,
        SystemNotificationType,
    };
    use crate::usecase::agent_session::status::{
        AgentStatusChanges, AgentStatusNotifier, TurnPhaseRepr,
    };
    use crate::usecase::workflow::ports::{
        WorkflowStallClearedNotification, WorkflowStallObservedNotification,
    };
    use std::future::Future;
    use std::path::Path;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Condvar, Mutex};
    use std::time::Duration;
    use tokio::sync::Notify;

    struct TokioSpawner;

    impl AgentTaskSpawner for TokioSpawner {
        fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
            tokio::spawn(future);
        }
    }

    struct DroppingSpawner;

    impl AgentTaskSpawner for DroppingSpawner {
        fn spawn(&self, _future: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {}
    }

    struct EmptyInstructionSource;

    impl InstructionSourcePort for EmptyInstructionSource {
        fn read_instruction_file(
            &self,
            _path: &Path,
            _worktree_root: &Path,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }

        fn instruction_cache_key(&self, _worktree_root: &Path) -> Option<String> {
            None
        }
    }

    fn test_session_runtime_locks() -> SessionRuntimeLocks {
        Arc::new(SessionCommandLocks::default())
    }

    fn accepted_queued_input(
        queue_item_id: &str,
        human_message_id: &str,
        turn_id: u64,
    ) -> QueuedTurnInput {
        let mut queued = QueuedTurnInput::new(
            queue_item_id.to_string(),
            PermissionMode::Edit,
            false,
            None,
            Vec::new(),
            "/repo".to_string(),
            Vec::new(),
            None,
        );
        queued.id = queue_item_id.to_string();
        queued.existing_human_message_id = Some(human_message_id.to_string());
        queued.existing_agent_message_id = Some(format!("{human_message_id}:agent"));
        queued.reserved_turn_id = Some(turn_id);
        queued.accepted_operation_id = Some(format!("operation-{turn_id}"));
        queued.execution_obligation_id = Some(format!("operation-{turn_id}.exec"));
        queued
    }

    #[test]
    fn accepted_effect_cache_defers_to_canonical_queue_order() {
        let canonical = vec![
            CanonicalQueuedSend {
                queue_item_id: "queue-2".to_string(),
                human_message_id: "human-2".to_string(),
                reserved_turn_id: "2".to_string(),
                input_ref: "input-2".to_string(),
            },
            CanonicalQueuedSend {
                queue_item_id: "queue-3".to_string(),
                human_message_id: "human-3".to_string(),
                reserved_turn_id: "3".to_string(),
                input_ref: "input-3".to_string(),
            },
        ];
        let mut pending = std::collections::HashMap::new();
        let later = accepted_queued_input("queue-3", "human-3", 3);
        cache_accepted_input_effect(&mut pending, later.clone(), &canonical).unwrap();
        assert!(
            next_cached_input_effect(&pending, &canonical).is_none(),
            "a later item restored alone must not satisfy the canonical front fence"
        );

        cache_accepted_input_effect(
            &mut pending,
            accepted_queued_input("queue-2", "human-2", 2),
            &canonical,
        )
        .unwrap();
        assert_eq!(
            next_cached_input_effect(&pending, &canonical).map(|queued| queued.id.as_str()),
            Some("queue-2")
        );
        assert_eq!(pending.len(), 2);

        cache_accepted_input_effect(&mut pending, later, &canonical).unwrap();
        assert_eq!(pending.len(), 2, "same-effect restoration must remain idempotent");
        assert!(cache_accepted_input_effect(
            &mut pending,
            accepted_queued_input("queue-4", "human-4", 4),
            &canonical,
        )
        .is_err());
    }

    #[tokio::test]
    async fn shutdown_admission_notifies_all_registered_idle_waiters() {
        let admission = Arc::new(ShutdownAdmission::default());
        let guard = admission.admit().unwrap();
        let first_waiter = admission.idle.notified();
        let second_waiter = admission.idle.notified();
        tokio::pin!(first_waiter);
        tokio::pin!(second_waiter);
        first_waiter.as_mut().enable();
        second_waiter.as_mut().enable();

        drop(guard);

        tokio::time::timeout(Duration::from_secs(1), async {
            first_waiter.await;
            second_waiter.await;
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn released_session_runtime_lock_is_pruned_on_the_next_acquire() {
        let locks = test_session_runtime_locks();
        let released = acquire_session_runtime_lock(&locks, "released").await;
        assert!(locks.contains_for_test("released").await);

        drop(released);
        let active = acquire_session_runtime_lock(&locks, "active").await;

        assert!(!locks.contains_for_test("released").await);
        assert!(locks.contains_for_test("active").await);
        drop(active);
    }

    #[test]
    fn dropping_session_runtime_lock_without_a_runtime_still_schedules_prune() {
        let locks = test_session_runtime_locks();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let released = runtime.block_on(acquire_session_runtime_lock(&locks, "released"));

        drop(runtime);
        drop(released);

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let active = acquire_session_runtime_lock(&locks, "active").await;
            assert!(!locks.contains_for_test("released").await);
            assert!(locks.contains_for_test("active").await);
            drop(active);
        });
    }

    #[tokio::test]
    async fn session_runtime_locks_serialize_one_session_and_keep_sessions_independent() {
        let locks = test_session_runtime_locks();
        let first = acquire_session_runtime_lock(&locks, "session-a").await;
        let waiter_locks = Arc::clone(&locks);
        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let guard = acquire_session_runtime_lock(&waiter_locks, "session-a").await;
                acquired_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                drop(guard);
            });
        });

        assert!(acquired_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(first);
        acquired_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        let other = acquire_session_runtime_lock(&locks, "session-b").await;
        assert!(
            locks.is_held_for_test("session-a"),
            "an actively held session lock must remain in the registry"
        );
        assert!(locks.contains_for_test("session-b").await);
        drop(other);

        release_tx.send(()).unwrap();
        waiter.join().unwrap();

        let final_guard = acquire_session_runtime_lock(&locks, "final").await;
        assert!(!locks.contains_for_test("session-a").await);
        drop(final_guard);
    }

    #[tokio::test]
    async fn repeated_session_runtime_locks_do_not_accumulate_registry_entries() {
        let locks = test_session_runtime_locks();

        for index in 0..100 {
            let guard = acquire_session_runtime_lock(&locks, &format!("session-{index}")).await;
            assert_eq!(locks.len_for_test().await, 1);
            drop(guard);
        }

        let final_guard = acquire_session_runtime_lock(&locks, "final").await;
        assert_eq!(locks.len_for_test().await, 1);
        assert!(locks.contains_for_test("final").await);
        drop(final_guard);
    }

    #[tokio::test]
    #[should_panic(expected = "session runtime lock re-entry is forbidden")]
    async fn session_runtime_lock_reentry_is_detected_in_tests() {
        let locks = test_session_runtime_locks();
        let _first = acquire_session_runtime_lock(&locks, "session-a").await;
        let _second = acquire_session_runtime_lock(&locks, "session-b").await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn session_runtime_lock_reentry_is_detected_on_a_multi_thread_runtime() {
        let locks = test_session_runtime_locks();
        let task = tokio::spawn(async move {
            let _first = acquire_session_runtime_lock(&locks, "session-a").await;
            tokio::task::yield_now().await;
            let _second = acquire_session_runtime_lock(&locks, "session-b").await;
        });

        let error = task.await.expect_err("re-entry must panic");
        assert!(error.is_panic());
    }

    #[tokio::test]
    async fn concurrently_polled_session_runtime_lock_acquires_detect_reentry() {
        let locks = test_session_runtime_locks();
        let holder_a_locks = Arc::clone(&locks);
        let holder_b_locks = Arc::clone(&locks);
        let (holder_a_ready_tx, holder_a_ready_rx) = tokio::sync::oneshot::channel();
        let (holder_b_ready_tx, holder_b_ready_rx) = tokio::sync::oneshot::channel();
        let (release_holder_a_tx, release_holder_a_rx) = tokio::sync::oneshot::channel();
        let (release_holder_b_tx, release_holder_b_rx) = tokio::sync::oneshot::channel();
        let holder_a = tokio::spawn(async move {
            let guard = acquire_session_runtime_lock(&holder_a_locks, "session-a").await;
            holder_a_ready_tx.send(()).unwrap();
            release_holder_a_rx.await.unwrap();
            drop(guard);
        });
        let holder_b = tokio::spawn(async move {
            let guard = acquire_session_runtime_lock(&holder_b_locks, "session-b").await;
            holder_b_ready_tx.send(()).unwrap();
            release_holder_b_rx.await.unwrap();
            drop(guard);
        });
        holder_a_ready_rx.await.unwrap();
        holder_b_ready_rx.await.unwrap();

        let reentry_locks = Arc::clone(&locks);
        let reentry = tokio::spawn(async move {
            let acquire_a = acquire_session_runtime_lock(&reentry_locks, "session-a");
            let acquire_b = acquire_session_runtime_lock(&reentry_locks, "session-b");
            tokio::join!(acquire_a, acquire_b)
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while !reentry.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("parallel re-entry must be detected before either session lock is released");

        release_holder_a_tx.send(()).unwrap();
        holder_a.await.unwrap();
        release_holder_b_tx.send(()).unwrap();
        holder_b.await.unwrap();

        let error = match reentry.await {
            Ok(_) => panic!("parallel re-entry must panic"),
            Err(error) => error,
        };
        assert!(error.is_panic());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn session_runtime_lock_drop_removes_task_ownership_from_another_thread() {
        let locks = test_session_runtime_locks();
        let task = tokio::spawn(async move {
            let first = acquire_session_runtime_lock(&locks, "session-a").await;
            tokio::task::spawn_blocking(move || drop(first))
                .await
                .unwrap();

            let second = acquire_session_runtime_lock(&locks, "session-b").await;
            drop(second);
        });

        task.await.unwrap();
    }

    #[tokio::test]
    #[should_panic(expected = "session runtime lock re-entry is forbidden")]
    async fn transferred_session_runtime_lock_detects_reentry_in_the_receiving_flow() {
        let locks = test_session_runtime_locks();
        let task_locks = Arc::clone(&locks);
        let mut first =
            tokio::spawn(
                async move { acquire_session_runtime_lock(&task_locks, "session-a").await },
            )
            .await
            .unwrap();
        first.adopt_for_current_test_flow();

        let _second = acquire_session_runtime_lock(&locks, "session-b").await;
    }

    #[tokio::test]
    async fn sequential_session_runtime_lock_acquires_are_not_reentry() {
        let locks = test_session_runtime_locks();
        let first = acquire_session_runtime_lock(&locks, "session-a").await;
        drop(first);

        let second = acquire_session_runtime_lock(&locks, "session-b").await;
        drop(second);
    }

    #[test]
    fn authoritative_snapshot_retry_preserves_backend_message_and_parts() {
        let message = crate::usecase::agent_session::event_log::session_error_message(
            "fatal-message".to_string(),
            "app server stopped".to_string(),
            42.0,
        );
        let parts = message.parts.clone().unwrap();
        let payload = PendingStreamDelta {
            message_id: message.id.clone(),
            seq: 1,
            snapshot: true,
            parts: parts.clone(),
            message: Some(message),
            authoritative: true,
        };
        let mut state = RuntimeSessionState::new("codex".to_string());

        assert!(on_authoritative_stream_emit_failure(&mut state, "session-1", &payload).is_some());

        let retry = state
            .authoritative_stream_retry_front()
            .expect("retry snapshot");
        assert_eq!(retry.parts, parts);
        let retry_message = retry.message.as_ref().expect("backend message metadata");
        assert_eq!(retry_message.id, "fatal-message");
        assert_eq!(retry_message.role, MessageRole::Agent);
        assert_eq!(retry_message.timestamp, 42.0);
    }

    #[test]
    fn authoritative_snapshot_supersedes_older_retry() {
        let mut state = RuntimeSessionState::new("codex".to_string());
        state.replace_coalesced_stream_retry(Some(PendingStreamDelta {
            message_id: "streaming-message".to_string(),
            seq: 1,
            snapshot: true,
            parts: vec![MessagePart::Text {
                content: "partial output".to_string(),
                parent_tool_use_id: None,
            }],
            message: None,
            authoritative: false,
        }));
        assert!(state.schedule_stream_flush());
        let message = crate::usecase::agent_session::event_log::session_error_message(
            "fatal-message".to_string(),
            "app server stopped".to_string(),
            42.0,
        );
        let payload = PendingStreamDelta {
            message_id: message.id.clone(),
            seq: 1,
            snapshot: true,
            parts: message.parts.clone().unwrap(),
            message: Some(message),
            authoritative: true,
        };

        prepare_authoritative_stream_emit(&mut state, &payload.message_id);
        assert!(!state.has_coalesced_stream_retry());
        assert!(!state.stream_flush_is_scheduled());
        assert!(on_authoritative_stream_emit_failure(&mut state, "session-1", &payload).is_some());

        let retry = state
            .authoritative_stream_retry_front()
            .expect("latest retry snapshot");
        assert_eq!(retry.message_id, "fatal-message");
        assert!(retry.message.is_some());
        assert!(retry.parts.iter().any(
            |part| matches!(part, MessagePart::Error { content, .. } if content == "app server stopped")
        ));
    }

    #[test]
    fn authoritative_snapshot_retry_coalesces_only_the_same_message_id() {
        let mut state = RuntimeSessionState::new("codex".to_string());
        let older = PendingStreamDelta {
            message_id: "fatal-message".to_string(),
            seq: 1,
            snapshot: true,
            parts: vec![MessagePart::Text {
                content: "older".to_string(),
                parent_tool_use_id: None,
            }],
            message: None,
            authoritative: true,
        };
        let newer = PendingStreamDelta {
            seq: 2,
            parts: vec![MessagePart::Text {
                content: "newer".to_string(),
                parent_tool_use_id: None,
            }],
            ..older.clone()
        };

        assert!(on_authoritative_stream_emit_failure(&mut state, "session-1", &older).is_some());
        prepare_authoritative_stream_emit(&mut state, &newer.message_id);
        state.clear_authoritative_stream_flush_schedule();
        assert!(on_authoritative_stream_emit_failure(&mut state, "session-1", &newer).is_some());

        assert_eq!(state.authoritative_stream_retry_count(), 1);
        let retry = state.authoritative_stream_retry_front().unwrap();
        assert_eq!(retry.seq, 2);
        assert!(matches!(
            retry.parts.as_slice(),
            [MessagePart::Text { content, .. }] if content == "newer"
        ));
    }

    #[test]
    fn workflow_execution_env_includes_run_and_node_execution_ids() {
        let context = crate::usecase::agent_session::session::workflow_node_context_mapper::to_dto(
            workflow_node_context(None, None, None),
        );

        assert_eq!(
            workflow_execution_env(Some(&context)),
            vec![
                (
                    "RELEASH_WORKFLOW_EXECUTION_ID".to_string(),
                    "run-1".to_string(),
                ),
                (
                    "RELEASH_NODE_EXECUTION_ID".to_string(),
                    "node-execution-1".to_string(),
                ),
            ]
        );
        assert!(workflow_execution_env(None).is_empty());
    }

    struct DispatchBackend {
        id: &'static str,
        model: &'static str,
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl AgentBackend for DispatchBackend {
        fn id(&self) -> &str {
            self.id
        }

        fn name(&self) -> &str {
            self.id
        }

        fn available_models(&self) -> Vec<ModelDescriptor> {
            vec![ModelDescriptor {
                id: ModelId::parse(self.model).unwrap(),
                display_name: self.model.to_string(),
            }]
        }

        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities { steering: false }
        }

        async fn open_session(
            &self,
            _spec: SessionSpec,
        ) -> Result<Box<dyn AgentSessionRuntime>, AgentBackendError> {
            Err(AgentBackendError::Other("not used".to_string()))
        }

        async fn skill_catalog(
            &self,
            _cwd: &Path,
            _query: Option<&str>,
            _limit: Option<usize>,
        ) -> Result<Vec<SkillEntry>, AgentBackendError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{}:skills", self.id));
            Ok(vec![SkillEntry {
                name: format!("{}-skill", self.id),
                description: "skill".to_string(),
                scope: self.id.to_string(),
            }])
        }

        async fn fuzzy_file_search(
            &self,
            _root: &Path,
            _query: &str,
            _limit: usize,
        ) -> Result<Option<Vec<String>>, AgentBackendError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{}:files", self.id));
            Ok(Some(vec![format!("{}-file", self.id)]))
        }
    }

    fn dispatch_test_usecase(
        data_dir: PathBuf,
        calls: Arc<Mutex<Vec<String>>>,
        default_id: &str,
    ) -> AgentSessionRuntimeUsecase {
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(DispatchBackend {
            id: "claude",
            model: "claude-opus-4-8",
            calls: Arc::clone(&calls),
        }));
        registry.register(Arc::new(DispatchBackend {
            id: "codex",
            model: "gpt-5.6-sol",
            calls,
        }));
        registry.set_default(Some(default_id.to_string()));
        let session_store = Arc::new(build_session_store());
        let workspace_query = crate::usecase::workspace_tree::TestWorkspaceQueryService::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        AgentSessionRuntimeUsecase::new(
            session_store,
            Arc::new(registry),
            Arc::new(AgentStatusCenter::new()),
            Arc::new(RecordingStatusNotifier::default()),
            Arc::new(RecordingAgentNotifier::default()),
            Arc::new(
                crate::adaptor::gateway::agent_session::runtime_projection::AgentRuntimeProjectionGatewayV1,
            ),
            Arc::new(TokioSpawner),
            None,
            Arc::new(EmptyInstructionSource),
            data_dir,
            workspace_query,
        )
    }

    #[derive(Default)]
    struct RecordingAgentNotifier {
        notices: Mutex<Vec<SessionNotice>>,
        state_changes: Mutex<Vec<AgentSessionStateChangedPayload>>,
        stall_observations: Mutex<Vec<AgentStallObservedPayload>>,
        stall_clears: Mutex<Vec<String>>,
        streaming_deltas: Mutex<Vec<AgentStreamingDeltaPayload>>,
        delivered_streaming_deltas: Mutex<Vec<AgentStreamingDeltaPayload>>,
        permission_modes: Mutex<Vec<(String, String)>>,
        model_updates: Mutex<Vec<(String, Vec<ModelInfo>, String)>>,
        display_windows: Mutex<Vec<GetSessionResponse>>,
        display_window_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
        streaming_delta_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
        fail_streaming_delta: Mutex<bool>,
        streaming_delta_outcomes: Mutex<std::collections::VecDeque<bool>>,
        event_order: Mutex<Vec<&'static str>>,
    }

    impl RecordingAgentNotifier {
        fn notices(&self) -> Vec<SessionNotice> {
            self.notices.lock().unwrap().clone()
        }

        fn state_changes(&self) -> Vec<AgentSessionStateChangedPayload> {
            self.state_changes.lock().unwrap().clone()
        }

        fn stall_observations(&self) -> Vec<AgentStallObservedPayload> {
            self.stall_observations.lock().unwrap().clone()
        }

        fn stall_clears(&self) -> Vec<String> {
            self.stall_clears.lock().unwrap().clone()
        }

        fn streaming_deltas(&self) -> Vec<AgentStreamingDeltaPayload> {
            self.streaming_deltas.lock().unwrap().clone()
        }

        fn delivered_streaming_deltas(&self) -> Vec<AgentStreamingDeltaPayload> {
            self.delivered_streaming_deltas.lock().unwrap().clone()
        }

        fn permission_modes(&self) -> Vec<(String, String)> {
            self.permission_modes.lock().unwrap().clone()
        }

        fn model_updates(&self) -> Vec<(String, Vec<ModelInfo>, String)> {
            self.model_updates.lock().unwrap().clone()
        }

        fn display_windows(&self) -> Vec<GetSessionResponse> {
            self.display_windows.lock().unwrap().clone()
        }

        fn set_display_window_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
            *self.display_window_hook.lock().unwrap() = Some(hook);
        }

        fn set_streaming_delta_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
            *self.streaming_delta_hook.lock().unwrap() = Some(hook);
        }

        fn set_streaming_delta_failure(&self, fail: bool) {
            *self.fail_streaming_delta.lock().unwrap() = fail;
        }

        fn set_streaming_delta_outcomes(&self, outcomes: impl IntoIterator<Item = bool>) {
            *self.streaming_delta_outcomes.lock().unwrap() = outcomes.into_iter().collect();
        }

        fn event_order(&self) -> Vec<&'static str> {
            self.event_order.lock().unwrap().clone()
        }
    }

    impl AgentSessionEventNotifier for RecordingAgentNotifier {
        fn persist_notice(&self, notice: SessionNotice) {
            self.notices.lock().unwrap().push(notice);
        }

        fn display_window_updated(&self, response: &GetSessionResponse) -> bool {
            if let Some(hook) = self.display_window_hook.lock().unwrap().clone() {
                hook();
            }
            self.display_windows.lock().unwrap().push(response.clone());
            self.event_order.lock().unwrap().push("display_window");
            true
        }

        fn session_state_changed(&self, payload: AgentSessionStateChangedPayload) {
            self.event_order.lock().unwrap().push("state_change");
            self.state_changes.lock().unwrap().push(payload);
        }

        fn stall_observed(&self, payload: AgentStallObservedPayload) {
            self.stall_observations.lock().unwrap().push(payload);
        }

        fn stall_cleared(&self, session_id: &str) {
            self.stall_clears
                .lock()
                .unwrap()
                .push(session_id.to_string());
        }

        fn streaming_delta(&self, payload: AgentStreamingDeltaPayload) -> bool {
            if let Some(hook) = self.streaming_delta_hook.lock().unwrap().clone() {
                hook();
            }
            let delivered = self
                .streaming_delta_outcomes
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| !*self.fail_streaming_delta.lock().unwrap());
            self.streaming_deltas.lock().unwrap().push(payload.clone());
            if delivered {
                self.delivered_streaming_deltas
                    .lock()
                    .unwrap()
                    .push(payload);
            }
            self.event_order.lock().unwrap().push("streaming_delta");
            delivered
        }

        fn supported_commands_updated(
            &self,
            _session_id: &str,
            _commands: Vec<crate::domain::agent_session::value_objects::SlashCommand>,
        ) {
        }

        fn token_usage_updated(
            &self,
            _session_id: &str,
            _token_usage: crate::usecase::agent_session::session::TokenUsage,
        ) {
        }

        fn permission_mode_changed(&self, session_id: &str, permission_mode: &str) {
            self.permission_modes
                .lock()
                .unwrap()
                .push((session_id.to_string(), permission_mode.to_string()));
        }

        fn models_updated(
            &self,
            session_id: &str,
            available_models: Vec<ModelInfo>,
            selected_model: String,
        ) {
            self.model_updates.lock().unwrap().push((
                session_id.to_string(),
                available_models,
                selected_model,
            ));
        }

        fn context_carry_updated(
            &self,
            _session_id: &str,
            _agent_session_id: Option<String>,
            _context_carry: Option<crate::usecase::agent_session::session::ContextCarryState>,
            _updated_at: f64,
        ) {
        }

        fn pending_message_consumed(
            &self,
            _session_id: &str,
            _queued_turn_id: Option<String>,
            _human_message: Option<ChatMessage>,
            _agent_message: ChatMessage,
        ) {
        }

        fn turn_prepared(
            &self,
            _session: &ChatSession,
            _human_message: &ChatMessage,
            _agent_message: &ChatMessage,
        ) {
        }
    }

    #[derive(Default)]
    struct RecordingStatusNotifier {
        changes: Mutex<Vec<AgentStatusChanges>>,
    }

    impl RecordingStatusNotifier {
        fn changes(&self) -> Vec<AgentStatusChanges> {
            self.changes.lock().unwrap().clone()
        }
    }

    impl AgentStatusNotifier for RecordingStatusNotifier {
        fn status_changed(&self, changes: AgentStatusChanges) {
            self.changes.lock().unwrap().push(changes);
        }
    }

    struct ReentrantWorkflowNotifier {
        usecase: Arc<AgentSessionRuntimeUsecase>,
        session_id: String,
        worktree_path: String,
        done: Arc<Notify>,
    }

    #[async_trait::async_trait]
    impl WorkflowTurnCompleteNotifier for ReentrantWorkflowNotifier {
        async fn turn_completed(&self, _notification: WorkflowTurnCompleteNotification) {
            let _ = self
                .usecase
                .send_message(SendAgentMessageRequest {
                    chat_session_id: Some(self.session_id.clone()),
                    worktree_path: self.worktree_path.clone(),
                    content: "repair".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    backend_id: Some("claude".to_string()),
                    model_id: None,
                    images: None,
                    mentions: None,
                    editor_context: None,
                })
                .await;
            self.done.notify_waiters();
        }
    }

    #[derive(Default)]
    struct RecordingWorkflowStallNotifier {
        notifications: Mutex<Vec<WorkflowStallObservedNotification>>,
        cleared_notifications: Mutex<Vec<WorkflowStallClearedNotification>>,
        stall_cleared_failures: Mutex<usize>,
        event_order: Mutex<Vec<&'static str>>,
        stall_observed_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
        stall_observed_record_delay: Mutex<Option<Duration>>,
    }

    impl RecordingWorkflowStallNotifier {
        fn notifications(&self) -> Vec<WorkflowStallObservedNotification> {
            self.notifications.lock().unwrap().clone()
        }

        fn cleared_notifications(&self) -> Vec<WorkflowStallClearedNotification> {
            self.cleared_notifications.lock().unwrap().clone()
        }

        fn fail_next_stall_cleared(&self) {
            *self.stall_cleared_failures.lock().unwrap() += 1;
        }

        fn event_order(&self) -> Vec<&'static str> {
            self.event_order.lock().unwrap().clone()
        }

        fn set_stall_observed_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
            *self.stall_observed_hook.lock().unwrap() = Some(hook);
        }

        fn set_stall_observed_record_delay(&self, delay: Duration) {
            *self.stall_observed_record_delay.lock().unwrap() = Some(delay);
        }
    }

    #[async_trait::async_trait]
    impl WorkflowStallNotifier for RecordingWorkflowStallNotifier {
        async fn stall_observed(&self, notification: WorkflowStallObservedNotification) {
            let hook = self.stall_observed_hook.lock().unwrap().clone();
            if let Some(hook) = hook {
                hook();
            }
            let delay = *self.stall_observed_record_delay.lock().unwrap();
            if let Some(delay) = delay {
                tokio::time::sleep(delay).await;
            }
            self.event_order.lock().unwrap().push("observed");
            self.notifications.lock().unwrap().push(notification);
        }

        async fn stall_cleared(
            &self,
            notification: WorkflowStallClearedNotification,
        ) -> Result<(), WorkflowError> {
            {
                let mut failures = self.stall_cleared_failures.lock().unwrap();
                if *failures > 0 {
                    *failures -= 1;
                    return Err(WorkflowError::external("injected workflow clear failure"));
                }
            }
            self.event_order.lock().unwrap().push("cleared");
            self.cleared_notifications
                .lock()
                .unwrap()
                .push(notification);
            Ok(())
        }
    }

    fn send_request(worktree_path: String) -> SendAgentMessageRequest {
        SendAgentMessageRequest {
            chat_session_id: None,
            worktree_path,
            content: "hello".to_string(),
            permission_mode: PermissionMode::Edit,
            plan_mode: false,
            backend_id: Some("claude".to_string()),
            model_id: None,
            images: None,
            mentions: None,
            editor_context: None,
        }
    }

    fn workflow_node_context(
        startup_timeout_secs: Option<u64>,
        startup_max_retries: Option<u32>,
        stale_timeout_secs: Option<u64>,
    ) -> WorkflowNodeContext {
        WorkflowNodeContext {
            execution_id: "run-1".to_string(),
            node_execution_id: "node-execution-1".to_string(),
            workflow_name: "workflow".to_string(),
            node_name: "step".to_string(),
            attempt: 0,
            parent_node_name: None,
            parent_attempt: None,
            order: 1,
            startup_timeout_secs,
            startup_max_retries,
            stale_timeout_secs,
        }
    }

    fn permission_request(id: &str) -> crate::domain::agent_session::entities::PermissionRequest {
        crate::domain::agent_session::entities::PermissionRequest {
            id: id.to_string(),
            tool_use_id: Some("toolu-1".to_string()),
            parent_tool_use_id: None,
            tool_name: "Bash".to_string(),
            body: crate::domain::agent_session::entities::PermissionRequestBody::ToolApproval {
                input: crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                    r#"{"command":"echo hi"}"#.to_string(),
                ),
            },
            title: None,
            display_name: None,
            description: None,
            decision_reason: None,
            status: PermissionRequestStatus::Pending,
        }
    }

    fn permission_request_msg(id: &str) -> PermissionRequestMsg {
        crate::adaptor::gateway::agent_session::runtime_projection::AgentRuntimeProjectionGatewayV1
            .pending_permission_request(&permission_request(id))
            .unwrap()
    }

    #[test]
    fn terminal_projection_maps_session_closed_to_idle_interruption() {
        let result = TurnResult::Interrupted {
            reason: DomainInterruptReason::SessionClosed,
            error: None,
        };
        let projection =
            crate::adaptor::gateway::agent_session::runtime_projection::AgentRuntimeProjectionGatewayV1
                .terminal_projection(
                    &result,
                    crate::domain::agent_session::aggregates::session::Session::terminal_outcome(
                        &result,
                    ),
                );

        assert_eq!(projection.exit_code, 0);
        assert!(projection.interrupted);
        assert_eq!(projection.session_state, SessionState::Idle);
        assert!(matches!(
            projection.event,
            TerminalEventProjection::Interrupted {
                reason: EventInterruptReason::SessionClosed,
                error: None,
            }
        ));
    }

    #[tokio::test]
    async fn close_session_finalizes_streaming_turn_and_persists_terminal_projection() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        let agent_message_id = response.agent_message.unwrap().id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "persisted prefix".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_persisted_text(
            &session_store,
            tmp.path(),
            &session_id,
            &agent_message_id,
            "persisted prefix",
        )
        .await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![
                    DomainMessagePart::Text {
                        content: "close tail".to_string(),
                        parent_tool_use_id: None,
                    },
                    DomainMessagePart::ToolUse {
                        id: "toolu-1".to_string(),
                        tool: "Task".to_string(),
                        input:
                            crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                                r#"{"run_in_background":true}"#.to_string(),
                            ),
                        parent_tool_use_id: None,
                    },
                    DomainMessagePart::ToolResult {
                        content: "background task launched".to_string(),
                        is_error: false,
                        tool_use_id: Some("toolu-1".to_string()),
                        parent_tool_use_id: None,
                        content_ref: None,
                        summary: None,
                    },
                    DomainMessagePart::ToolUse {
                        id: "toolu-2".to_string(),
                        tool: "Read".to_string(),
                        input:
                            crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                                "{}".to_string(),
                            ),
                        parent_tool_use_id: None,
                    },
                ]),
            )
            .unwrap();
        wait_for_streaming_text(&usecase, &session_id, "close tail").await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-close")),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;
        let before_close_parts =
            persisted_message_parts(&session_store, tmp.path(), &session_id, &agent_message_id);
        assert!(before_close_parts.iter().any(|part| matches!(
            part,
            MessagePart::Text { content, .. } if content.contains("persisted prefix")
        )));
        assert!(before_close_parts.iter().any(|part| matches!(
            part,
            MessagePart::Text { content, .. } if content.contains("close tail")
        )));
        assert!(before_close_parts.iter().any(|part| matches!(
            part,
            MessagePart::Permission {
                status: PermissionPartStatus::Pending,
                ..
            }
        )));

        usecase.close_session(&session_id).await.unwrap();

        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::SessionClosed,
                exit_code: 0,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::ToolCallFailed { tool_use_id, .. } if tool_use_id == "toolu-2"
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TaskStatusChanged {
                task_tool_use_id,
                ..
            } if task_tool_use_id == "toolu-1"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::PermissionResolved {
                request_id: Some(request_id),
                decision: crate::usecase::agent_session::event_log::PermissionDecision::Cancelled,
                ..
            } if request_id == "perm-close"
        )));
        assert!(latest_unresolved_permission_request(&events).is_none());
        let projected = TurnEventLog::from_events(events).project();
        assert_eq!(projected.status.session_state, SessionState::Idle);
        assert_eq!(projected.status.turn_phase, TurnPhase::Idle);

        let reopened = usecase
            .get_session(&session_id)
            .await
            .unwrap()
            .expect("reopened session");
        assert_eq!(reopened.turn_phase, TurnPhase::Idle);
        assert!(reopened.pending_permission_request.is_none());
        assert_eq!(
            reopened.last_turn_interruption,
            Some(crate::usecase::agent_session::session::TurnInterruption {
                message_id: agent_message_id.clone(),
                reason:
                    crate::usecase::agent_session::session::TurnInterruptionReason::SessionClosed,
            })
        );
        let parts = reopened
            .session
            .messages
            .iter()
            .find(|message| message.id == agent_message_id)
            .and_then(|message| message.parts.as_ref())
            .expect("persisted agent parts");
        assert!(parts.iter().any(|part| matches!(
            part,
            MessagePart::Text { content, .. } if content.contains("close tail")
        )));
        assert!(parts.iter().any(|part| matches!(
            part,
            MessagePart::ToolResult {
                tool_use_id: Some(tool_use_id),
                is_error: true,
                ..
            } if tool_use_id == "toolu-2"
        )));
        assert!(parts.iter().any(|part| matches!(
            part,
            MessagePart::ToolResult {
                tool_use_id: Some(tool_use_id),
                is_error: false,
                ..
            } if tool_use_id == "toolu-1"
        )));
        assert!(!parts.iter().any(|part| matches!(
            part,
            MessagePart::TaskStatus {
                task_tool_use_id,
                ..
            } if task_tool_use_id == "toolu-1"
        )));
        assert!(parts.iter().any(|part| matches!(
            part,
            MessagePart::Permission {
                status: PermissionPartStatus::Cancelled,
                ..
            }
        )));
        assert!(controller
            .call_kinds_for(&session_id)
            .contains(&TestRuntimeCallKind::Close));
    }

    #[tokio::test]
    async fn close_session_without_active_turn_does_not_create_interruption() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        let events_before = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();

        usecase.close_session(&session.id).await.unwrap();

        assert_eq!(
            session_store
                .load_session_events(tmp.path(), &session.id)
                .unwrap(),
            events_before
        );
        assert!(controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Close));
    }

    #[tokio::test]
    async fn close_session_keeps_runtime_state_when_force_flush_fails_and_can_retry() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let fail_once = Arc::new(std::sync::atomic::AtomicBool::new(true));
        session_store.set_persist_parts_hook_for_test({
            let fail_once = Arc::clone(&fail_once);
            Arc::new(move |_, _, _| {
                if fail_once.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    Err("injected close flush failure".to_string())
                } else {
                    Ok(())
                }
            })
        });

        let error = usecase.close_session(&session_id).await.unwrap_err();

        assert!(error.to_string().contains("injected close flush failure"));
        assert!(usecase.has_live_runtime(&session_id).await);
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );
        assert!(!controller
            .call_kinds_for(&session_id)
            .contains(&TestRuntimeCallKind::Close));
        assert!(!session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::TurnInterrupted { .. })));

        usecase.close_session(&session_id).await.unwrap();
        assert!(!usecase.has_live_runtime(&session_id).await);
    }

    #[tokio::test]
    async fn close_all_failure_reopens_admission_and_session_transition() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let failed_session_id = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap()
            .session
            .id;
        let successful_session_id = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap()
            .session
            .id;
        let fail_once = Arc::new(AtomicBool::new(true));
        session_store.set_persist_parts_hook_for_test({
            let fail_once = Arc::clone(&fail_once);
            let failed_session_id = failed_session_id.clone();
            Arc::new(move |session_id, _, _| {
                if session_id == failed_session_id && fail_once.swap(false, Ordering::SeqCst) {
                    Err("injected application close flush failure".to_string())
                } else {
                    Ok(())
                }
            })
        });

        let error = usecase.close_all().await.unwrap_err();

        assert!(error
            .to_string()
            .contains("injected application close flush failure"));
        assert!(error.to_string().contains(&failed_session_id));
        assert!(!usecase.has_live_runtime(&successful_session_id).await);
        assert!(controller
            .call_kinds_for(&successful_session_id)
            .contains(&TestRuntimeCallKind::Close));
        assert_eq!(
            session_store
                .load_session_events(tmp.path(), &successful_session_id)
                .unwrap()
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::TurnInterrupted {
                        reason: EventInterruptReason::SessionClosed,
                        ..
                    }
                ))
                .count(),
            1
        );
        assert!(usecase.has_live_runtime(&failed_session_id).await);
        assert!(!session_store
            .load_session_events(tmp.path(), &failed_session_id)
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::TurnInterrupted { .. })));
        assert!(!usecase.ctx.shutdown_admission.is_shutting_down());
        assert!(
            !usecase
                .ctx
                .sessions
                .lock()
                .await
                .get(&failed_session_id)
                .expect("failed session remains")
                .is_closing()
        );
        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(failed_session_id),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "accepted after failed application shutdown".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn close_all_finalizes_every_active_session_once() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let mut session_ids = Vec::new();
        for _ in 0..2 {
            session_ids.push(
                usecase
                    .send_message(send_request(tmp.path().to_string_lossy().to_string()))
                    .await
                    .unwrap()
                    .session
                    .id,
            );
        }

        usecase.close_all().await.unwrap();

        for session_id in session_ids {
            assert!(!usecase.has_live_runtime(&session_id).await);
            assert!(controller
                .call_kinds_for(&session_id)
                .contains(&TestRuntimeCallKind::Close));
            assert_eq!(
                session_store
                    .load_session_events(tmp.path(), &session_id)
                    .unwrap()
                    .iter()
                    .filter(|event| matches!(
                        event,
                        AgentSessionEvent::TurnInterrupted {
                            reason: EventInterruptReason::SessionClosed,
                            ..
                        }
                    ))
                    .count(),
                1
            );
        }
    }

    #[tokio::test]
    async fn close_session_appends_terminal_batch_atomically_and_can_retry() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let terminal_failures = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session_store.set_append_event_hook_for_test({
            let terminal_failures = Arc::clone(&terminal_failures);
            Arc::new(move |_, event| {
                if matches!(event, AgentSessionEvent::TurnInterrupted { .. })
                    && terminal_failures.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                        < PERSIST_MAX_ATTEMPTS
                {
                    Err("injected terminal append failure".to_string())
                } else {
                    Ok(())
                }
            })
        });

        let error = usecase.close_session(&session_id).await.unwrap_err();

        assert!(error
            .to_string()
            .contains("injected terminal append failure"));
        assert!(usecase.has_live_runtime(&session_id).await);
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );
        assert!(!controller
            .call_kinds_for(&session_id)
            .contains(&TestRuntimeCallKind::Close));
        let failed_events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(!failed_events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::FinalPartsRecorded { .. }
                | AgentSessionEvent::TurnInterrupted { .. }
        )));

        usecase.close_session(&session_id).await.unwrap();
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::TurnInterrupted { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn close_session_rolls_back_terminal_when_message_persist_fails_and_can_retry() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let agent_message_id = response.agent_message.unwrap().id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "text committed before terminal".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_streaming_text(&usecase, &session_id, "text committed before terminal").await;
        let persist_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session_store.set_persist_parts_hook_for_test({
            let persist_count = Arc::clone(&persist_count);
            Arc::new(move |_, _, _| {
                let call = persist_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if (2..2 + PERSIST_MAX_ATTEMPTS).contains(&call) {
                    Err("injected final message persist failure".to_string())
                } else {
                    Ok(())
                }
            })
        });

        let error = usecase.close_session(&session_id).await.unwrap_err();

        assert!(error
            .to_string()
            .contains("injected final message persist failure"));
        assert!(usecase.has_live_runtime(&session_id).await);
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );
        assert!(!controller
            .call_kinds_for(&session_id)
            .contains(&TestRuntimeCallKind::Close));
        let events_after_terminal = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            events_after_terminal
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::TurnInterrupted { .. }))
                .count(),
            0
        );

        usecase.close_session(&session_id).await.unwrap();
        assert!(!usecase.has_live_runtime(&session_id).await);
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::TurnInterrupted { .. }))
                .count(),
            1
        );
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::PermissionRequested { request, .. }
                if request.id == "late-permission"
        )));
        let persisted_parts =
            persisted_message_parts(&session_store, tmp.path(), &session_id, &agent_message_id);
        assert!(persisted_parts.iter().any(|part| matches!(
            part,
            MessagePart::Text { content, .. } if content.contains("text committed before terminal")
        )));
    }

    #[tokio::test]
    async fn set_session_backend_finalizes_active_turn_before_runtime_close() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;

        let switched = usecase
            .set_session_backend(&session_id, "codex")
            .await
            .unwrap();

        assert_eq!(switched.session.backend_id.as_deref(), Some("codex"));
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::SessionClosed,
                ..
            }
        )));
        assert!(controller
            .call_kinds_for(&session_id)
            .contains(&TestRuntimeCallKind::Close));
    }

    #[tokio::test]
    async fn set_session_backend_serializes_competing_send_with_runtime_transition() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let switch_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            let session_id = session_id.clone();
            async move { usecase.set_session_backend(&session_id, "codex").await }
        });
        wait_for_session_closing(&usecase, &session_id).await;

        let during_transition = session_store
            .get_session_meta(tmp.path(), &session_id)
            .unwrap()
            .unwrap();
        assert_eq!(during_transition.backend_id, "claude");
        let send_error = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "competing backend send".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap_err();
        assert!(send_error.to_string().contains("Agent session is closing"));

        let switched = switch_task.await.unwrap().unwrap();
        assert_eq!(switched.session.backend_id.as_deref(), Some("codex"));
        assert!(!usecase.has_live_runtime(&session_id).await);
        let resumed = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "send after backend switch".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        assert_eq!(resumed.session.backend_id.as_deref(), Some("codex"));
        let sessions = usecase.ctx.sessions.lock().await;
        assert_eq!(
            sessions
                .get(&session_id)
                .map(|state| state.backend_id.as_str()),
            Some("codex")
        );
    }

    #[tokio::test]
    async fn close_all_finalizes_active_turn_for_fresh_runtime_restore() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let agent_message_id = response.agent_message.unwrap().id;
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "persisted prefix".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_persisted_text(
            &session_store,
            tmp.path(),
            &session_id,
            &agent_message_id,
            "persisted prefix",
        )
        .await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![
                    DomainMessagePart::Text {
                        content: "shutdown tail".to_string(),
                        parent_tool_use_id: None,
                    },
                    DomainMessagePart::ToolUse {
                        id: "toolu-shutdown".to_string(),
                        tool: "Task".to_string(),
                        input:
                            crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                                r#"{"run_in_background":true}"#.to_string(),
                            ),
                        parent_tool_use_id: None,
                    },
                    DomainMessagePart::ToolResult {
                        content: "background task launched".to_string(),
                        is_error: false,
                        tool_use_id: Some("toolu-shutdown".to_string()),
                        parent_tool_use_id: None,
                        content_ref: None,
                        summary: None,
                    },
                ]),
            )
            .unwrap();
        wait_for_streaming_text(&usecase, &session_id, "shutdown tail").await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-shutdown")),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;
        let before_shutdown_parts =
            persisted_message_parts(&session_store, tmp.path(), &session_id, &agent_message_id);
        assert!(before_shutdown_parts.iter().any(|part| matches!(
            part,
            MessagePart::Text { content, .. } if content.contains("persisted prefix")
        )));
        assert!(before_shutdown_parts.iter().any(|part| matches!(
            part,
            MessagePart::Text { content, .. } if content.contains("shutdown tail")
        )));
        assert!(before_shutdown_parts.iter().any(|part| matches!(
            part,
            MessagePart::Permission {
                status: PermissionPartStatus::Pending,
                ..
            }
        )));

        usecase.close_all().await.unwrap();
        drop(usecase);
        let restarted =
            crate::test_support::build_agent_runtime_usecase(session_store.clone(), tmp.path());

        let reopened = restarted
            .get_session(&session_id)
            .await
            .unwrap()
            .expect("restored session");
        assert_eq!(reopened.turn_phase, TurnPhase::Idle);
        assert_eq!(
            reopened.last_turn_interruption,
            Some(crate::usecase::agent_session::session::TurnInterruption {
                message_id: agent_message_id.clone(),
                reason:
                    crate::usecase::agent_session::session::TurnInterruptionReason::SessionClosed,
            })
        );
        assert!(reopened.pending_permission_request.is_none());
        assert!(reopened.session.messages.iter().any(|message| {
            message.parts.as_ref().is_some_and(|parts| {
                parts.iter().any(|part| {
                    matches!(
                        part,
                        MessagePart::Text { content, .. } if content.contains("shutdown tail")
                    )
                })
            })
        }));
        let parts = reopened
            .session
            .messages
            .iter()
            .find(|message| message.id == agent_message_id)
            .and_then(|message| message.parts.as_ref())
            .expect("persisted shutdown parts");
        assert!(!parts.iter().any(|part| matches!(
            part,
            MessagePart::TaskStatus {
                task_tool_use_id,
                ..
            } if task_tool_use_id == "toolu-shutdown"
        )));
        assert!(parts.iter().any(|part| matches!(
            part,
            MessagePart::Permission {
                status: PermissionPartStatus::Cancelled,
                ..
            }
        )));
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::SessionClosed,
                ..
            }
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TaskStatusChanged {
                task_tool_use_id,
                ..
            } if task_tool_use_id == "toolu-shutdown"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::PermissionResolved {
                request_id: Some(request_id),
                decision: crate::usecase::agent_session::event_log::PermissionDecision::Cancelled,
                ..
            } if request_id == "perm-shutdown"
        )));
        assert!(latest_unresolved_permission_request(&events).is_none());
        assert!(controller
            .call_kinds_for(&session_id)
            .contains(&TestRuntimeCallKind::Close));
    }

    #[tokio::test]
    async fn close_session_drains_competing_backend_completion_without_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let close_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            let session_id = session_id.clone();
            async move { usecase.close_session(&session_id).await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!close_task.is_finished());

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        close_task.await.unwrap().unwrap();

        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::TurnCompleted { .. })));
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::SessionClosed,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn close_session_waits_for_competing_send_message_before_finalizing() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        controller.pause_start_turn();
        let send_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            let session_id = session.id.clone();
            let worktree_path = tmp.path().to_string_lossy().to_string();
            async move {
                usecase
                    .send_message(SendAgentMessageRequest {
                        chat_session_id: Some(session_id),
                        worktree_path,
                        content: "competing send".to_string(),
                        permission_mode: PermissionMode::Edit,
                        plan_mode: false,
                        backend_id: Some("claude".to_string()),
                        model_id: None,
                        images: None,
                        mentions: None,
                        editor_context: None,
                    })
                    .await
            }
        });
        wait_for_call(&controller, &session.id, TestRuntimeCallKind::StartTurn).await;

        let close_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            let session_id = session.id.clone();
            async move { usecase.close_session(&session_id).await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!close_task.is_finished());

        controller.release_start_turn();
        send_task.await.unwrap().unwrap();
        close_task.await.unwrap().unwrap();

        assert!(!usecase.has_live_runtime(&session.id).await);
        assert!(session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::TurnInterrupted {
                    reason: EventInterruptReason::SessionClosed,
                    ..
                }
            )));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocked_turn_start_does_not_block_other_session_and_remains_terminalizable() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Arc::new(FileSessionStorage::default());
        let session_store = Arc::new(SessionStore::new(storage.clone()));
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: None,
            },
        )
        .unwrap();
        let unrelated_session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        usecase
            .start_session(
                &unrelated_session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        storage.reset_event_read_count();
        let hook_entered = Arc::new(Barrier::new(2));
        let release_hook = Arc::new(Barrier::new(2));
        let global_registry_was_available = Arc::new(AtomicBool::new(false));
        session_store.set_appended_event_hook_for_test({
            let sessions = Arc::clone(&usecase.ctx.sessions);
            let session_id = session.id.clone();
            let hook_entered = Arc::clone(&hook_entered);
            let release_hook = Arc::clone(&release_hook);
            let global_registry_was_available = Arc::clone(&global_registry_was_available);
            Arc::new(move |event_session_id, event| {
                if event_session_id == session_id
                    && matches!(event, AgentSessionEvent::TurnStarted { .. })
                {
                    global_registry_was_available
                        .store(sessions.try_lock().is_ok(), Ordering::SeqCst);
                    hook_entered.wait();
                    release_hook.wait();
                }
            })
        });
        controller.pause_start_turn();
        let start_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            let session_id = session.id.clone();
            async move {
                let _session_guard = usecase.acquire_session_lock(&session_id).await;
                usecase
                    .start_turn_locked(
                        &session_id,
                        PermissionMode::Edit,
                        "cancel during durable start".to_string(),
                        None,
                        Vec::new(),
                    )
                    .await
            }
        });

        hook_entered.wait();
        assert_eq!(storage.event_read_count(), 0);
        tokio::time::timeout(
            Duration::from_secs(1),
            usecase.close_session(&unrelated_session.id),
        )
        .await
        .expect("unrelated session close must not wait for another session's append")
        .unwrap();
        let global_registry_guard = usecase.ctx.sessions.lock().await;
        start_task.abort();
        release_hook.wait();
        assert!(start_task.await.unwrap_err().is_cancelled());
        drop(global_registry_guard);
        assert!(global_registry_was_available.load(Ordering::SeqCst));

        usecase.close_all().await.unwrap();

        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::TurnInterrupted {
                        reason: EventInterruptReason::SessionClosed,
                        ..
                    }
                ))
                .count(),
            1
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn close_all_waits_for_admitted_send_before_snapshotting_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        let hook_entered = Arc::new(Barrier::new(2));
        let release_hook = Arc::new(Barrier::new(2));
        let blocked = Arc::new(AtomicBool::new(false));
        session_store.set_append_event_hook_for_test({
            let session_id = session.id.clone();
            let hook_entered = Arc::clone(&hook_entered);
            let release_hook = Arc::clone(&release_hook);
            let blocked = Arc::clone(&blocked);
            Arc::new(move |event_session_id, event| {
                if event_session_id == session_id
                    && matches!(event, AgentSessionEvent::TurnStarted { .. })
                    && !blocked.swap(true, Ordering::SeqCst)
                {
                    hook_entered.wait();
                    release_hook.wait();
                }
                Ok(())
            })
        });
        controller.pause_start_turn();
        let send_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            let session_id = session.id.clone();
            let worktree_path = tmp.path().to_string_lossy().to_string();
            async move {
                usecase
                    .send_message(SendAgentMessageRequest {
                        chat_session_id: Some(session_id),
                        worktree_path,
                        content: "admitted before shutdown".to_string(),
                        permission_mode: PermissionMode::Edit,
                        plan_mode: false,
                        backend_id: Some("claude".to_string()),
                        model_id: None,
                        images: None,
                        mentions: None,
                        editor_context: None,
                    })
                    .await
            }
        });

        hook_entered.wait();
        let close_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            async move { usecase.close_all().await }
        });
        let shutdown_deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !usecase.ctx.shutdown_admission.is_shutting_down() {
            assert!(std::time::Instant::now() < shutdown_deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!close_task.is_finished());
        release_hook.wait();

        let send_error = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "rejected after shutdown".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap_err();
        assert!(send_error.to_string().contains("runtime is shutting down"));
        let start_error = usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap_err();
        assert!(start_error.to_string().contains("runtime is shutting down"));

        wait_for_call(&controller, &session.id, TestRuntimeCallKind::StartTurn).await;
        assert!(!close_task.is_finished());
        controller.release_start_turn();
        send_task.await.unwrap().unwrap();
        close_task.await.unwrap().unwrap();

        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::SessionClosed,
                ..
            }
        )));
        assert!(controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Close));
    }

    #[tokio::test]
    async fn close_session_waits_for_competing_permission_response_before_finalizing() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-close-race")),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;
        controller.pause_respond_permission();
        let response_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            let session_id = session_id.clone();
            async move {
                usecase
                    .respond_permission(
                        &session_id,
                        PermissionResponse {
                            request_id: "perm-close-race".to_string(),
                            decision: PermissionResponseDecision::Allow {
                                updated_input: None,
                                answers: None,
                            },
                        },
                    )
                    .await
            }
        });
        wait_for_call(
            &controller,
            &session_id,
            TestRuntimeCallKind::RespondPermission {
                request_id: "perm-close-race".to_string(),
            },
        )
        .await;

        let close_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            let session_id = session_id.clone();
            async move { usecase.close_session(&session_id).await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!close_task.is_finished());

        controller.release_respond_permission();
        response_task.await.unwrap().unwrap();
        close_task.await.unwrap().unwrap();

        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::PermissionResolved {
                request_id: Some(request_id),
                decision: crate::usecase::agent_session::event_log::PermissionDecision::Allowed,
                ..
            } if request_id == "perm-close-race"
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::PermissionResolved {
                request_id: Some(request_id),
                decision: crate::usecase::agent_session::event_log::PermissionDecision::Cancelled,
                ..
            } if request_id == "perm-close-race"
        )));
    }

    #[tokio::test]
    async fn close_session_rejects_new_send_while_event_drain_is_active() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            build_agent_runtime_usecase_with_controller(session_store, tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let close_task = tokio::spawn({
            let usecase = Arc::clone(&usecase);
            let session_id = session_id.clone();
            async move { usecase.close_session(&session_id).await }
        });
        wait_for_session_closing(&usecase, &session_id).await;

        let error = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "too late".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap_err();

        assert!(error.to_string().contains("Agent session is closing"));
        close_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn get_session_reads_interruption_projection_without_loading_long_event_log() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Arc::new(FileSessionStorage::default());
        let session_store = Arc::new(SessionStore::new(storage.clone()));
        let (usecase, _controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session_store
            .append_session_event_and_project_state(
                tmp.path(),
                &session.id,
                AgentSessionEvent::TurnStarted {
                    turn_id: 1,
                    message_id: "human-long".to_string(),
                    assistant_message_id: Some("agent-long".to_string()),
                    prompt: PromptInput::default(),
                    at: 1.0,
                },
            )
            .unwrap();
        for index in 0..500 {
            session_store
                .append_session_event_without_projection(
                    tmp.path(),
                    &session.id,
                    AgentSessionEvent::TextRecorded {
                        turn_id: 1,
                        message_id: "agent-long".to_string(),
                        content: format!("chunk-{index}"),
                        parent_tool_use_id: None,
                    },
                )
                .unwrap();
        }
        session_store
            .append_session_event_and_project_state(
                tmp.path(),
                &session.id,
                AgentSessionEvent::TurnInterrupted {
                    turn_id: 1,
                    reason: EventInterruptReason::SessionClosed,
                    exit_code: 0,
                    error: None,
                },
            )
            .unwrap();
        storage.reset_event_read_count();

        let response = usecase
            .get_session(&session.id)
            .await
            .unwrap()
            .expect("session response");

        assert_eq!(
            response.last_turn_interruption,
            Some(crate::usecase::agent_session::session::TurnInterruption {
                message_id: "agent-long".to_string(),
                reason:
                    crate::usecase::agent_session::session::TurnInterruptionReason::SessionClosed,
            })
        );
        assert_eq!(storage.event_read_count(), 0);
    }

    #[tokio::test]
    async fn display_session_window_is_bounded_by_the_backend_retention_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        for index in 0..210 {
            add_message_internal(
                &session_store,
                tmp.path(),
                &session.id,
                MessageRole::Human,
                &format!("message-{index}"),
                None,
                None,
            )
            .unwrap();
        }

        let response = usecase
            .get_display_session_window(&session.id, Some(usize::MAX))
            .await
            .unwrap()
            .expect("display window");

        assert_eq!(response.session.messages.len(), RETAINED_MESSAGE_CAP);
        assert_eq!(response.session.messages[0].content, "message-10");
        assert_eq!(
            response.session.messages.last().unwrap().content,
            "message-209"
        );
        assert_eq!(
            response.initial_page.unwrap().total_count,
            210,
            "the bounded body must retain full-history page accounting"
        );
    }

    #[tokio::test]
    async fn display_session_window_is_published_inside_the_runtime_event_ordering_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let (usecase, _controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        let session_locks = usecase.ctx.session_locks.clone();
        let session_id = session.id.clone();
        event_notifier.set_display_window_hook(Arc::new(move || {
            assert!(
                session_locks.is_held_for_test(&session_id),
                "the bounded read must publish before a later runtime event can acquire the session"
            );
        }));

        let response = usecase
            .get_display_session_window(&session.id, None)
            .await
            .unwrap()
            .expect("display window");

        let published = event_notifier.display_windows();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].session.id, response.session.id);
        assert_eq!(event_notifier.event_order(), vec!["display_window"]);
    }

    #[tokio::test]
    async fn display_session_window_overlays_the_latest_runtime_stream_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            build_agent_runtime_usecase_with_controller(session_store, tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let agent_message_id = response.agent_message.unwrap().id;
        let live_parts = vec![MessagePart::Text {
            content: "latest live snapshot".to_string(),
            parent_tool_use_id: None,
        }];
        {
            let mut sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get_mut(&session_id).expect("live runtime state");
            assert_eq!(
                state.streaming_message_id.as_deref(),
                Some(agent_message_id.as_str())
            );
            state.restore_stream_buffer_for_test(Vec::new(), live_parts.clone(), false);
            state.observe_emitted_stream_sequence(7);
        }

        let window = usecase
            .get_display_session_window(&session_id, None)
            .await
            .unwrap()
            .expect("display window");
        let displayed = window
            .session
            .messages
            .iter()
            .find(|message| message.id == agent_message_id)
            .expect("streaming message");

        assert_eq!(displayed.parts.as_ref(), Some(&live_parts));
        assert_eq!(displayed.streaming_final_seq, 7);
    }

    #[tokio::test]
    async fn get_session_returns_in_memory_pending_permission_request() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-1")),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;

        let loaded = usecase
            .get_session(&session_id)
            .await
            .unwrap()
            .expect("session");

        assert_eq!(loaded.turn_phase, TurnPhase::WaitingPermission);
        assert_eq!(
            loaded
                .pending_permission_request
                .as_ref()
                .map(|r| r.id.as_str()),
            Some("perm-1")
        );
        assert!(loaded.pending_permission_state_revision > 0);
    }

    #[tokio::test]
    async fn get_session_ignores_event_log_pending_when_runtime_state_is_clear() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session_store
            .append_session_event_without_projection(
                tmp.path(),
                &session.id,
                AgentSessionEvent::TurnStarted {
                    turn_id: 1,
                    message_id: "human-1".to_string(),
                    assistant_message_id: Some("agent-1".to_string()),
                    prompt: PromptInput {
                        content: "run".to_string(),
                        mentions: Vec::new(),
                        attachment_refs: Vec::new(),
                        parts: Vec::new(),
                    },
                    at: 1.0,
                },
            )
            .unwrap();
        session_store
            .append_session_event_without_projection(
                tmp.path(),
                &session.id,
                AgentSessionEvent::PermissionRequested {
                    turn_id: 1,
                    tool_use_id: Some("toolu-1".to_string()),
                    request: crate::usecase::agent_session::runtime::event_apply::pending_permission_request_from_msg(
                        &permission_request_msg("perm-from-log"),
                    )
                    .unwrap(),
                },
            )
            .unwrap();
        usecase
            .insert_runtime_state_for_test(&session.id, TurnPhase::Idle, false)
            .await;

        let loaded = usecase
            .get_session(&session.id)
            .await
            .unwrap()
            .expect("session");

        assert_eq!(loaded.turn_phase, TurnPhase::Idle);
        assert!(loaded.pending_permission_request.is_none());
        let presented = usecase
            .find_permission_request(&session.id, "perm-from-log")
            .await
            .unwrap()
            .expect("permission request");
        assert_eq!(presented.id, "perm-from-log");
        let sessions = usecase.ctx.sessions.lock().await;
        let state = sessions.get(&session.id).expect("runtime state");
        assert_eq!(state.projected_turn_phase(), TurnPhase::Idle);
        assert!(state.permission_request_cache.is_none());
    }

    #[tokio::test]
    async fn get_session_does_not_publish_event_log_permission_without_live_runtime() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session_store
            .append_session_event_without_projection(
                tmp.path(),
                &session.id,
                AgentSessionEvent::TurnStarted {
                    turn_id: 1,
                    message_id: "human-1".to_string(),
                    assistant_message_id: Some("agent-1".to_string()),
                    prompt: PromptInput {
                        content: "run".to_string(),
                        mentions: Vec::new(),
                        attachment_refs: Vec::new(),
                        parts: Vec::new(),
                    },
                    at: 1.0,
                },
            )
            .unwrap();
        session_store
            .append_session_event_without_projection(
                tmp.path(),
                &session.id,
                AgentSessionEvent::PermissionRequested {
                    turn_id: 1,
                    tool_use_id: Some("toolu-1".to_string()),
                    request: crate::usecase::agent_session::runtime::event_apply::pending_permission_request_from_msg(
                        &permission_request_msg("perm-from-log"),
                    )
                    .unwrap(),
                },
            )
            .unwrap();

        let loaded = usecase
            .get_session(&session.id)
            .await
            .unwrap()
            .expect("session");

        assert_eq!(loaded.turn_phase, TurnPhase::Idle);
        assert!(loaded.pending_permission_request.is_none());
    }

    #[tokio::test]
    async fn respond_permission_resolves_event_log_only_pending_permission() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session_store
            .append_session_event_without_projection(
                tmp.path(),
                &session.id,
                AgentSessionEvent::TurnStarted {
                    turn_id: 1,
                    message_id: "human-1".to_string(),
                    assistant_message_id: Some("agent-1".to_string()),
                    prompt: PromptInput {
                        content: "run".to_string(),
                        mentions: Vec::new(),
                        attachment_refs: Vec::new(),
                        parts: Vec::new(),
                    },
                    at: 1.0,
                },
            )
            .unwrap();
        session_store
            .append_session_event_without_projection(
                tmp.path(),
                &session.id,
                AgentSessionEvent::PermissionRequested {
                    turn_id: 1,
                    tool_use_id: Some("toolu-1".to_string()),
                    request: crate::usecase::agent_session::runtime::event_apply::pending_permission_request_from_msg(
                        &permission_request_msg("perm-from-log"),
                    )
                    .unwrap(),
                },
            )
            .unwrap();
        usecase
            .insert_failing_runtime_state_for_test(&session.id)
            .await;

        usecase
            .respond_permission(
                &session.id,
                PermissionResponse {
                    request_id: "perm-from-log".to_string(),
                    decision: PermissionResponseDecision::Allow {
                        updated_input: None,
                        answers: None,
                    },
                },
            )
            .await
            .unwrap();

        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::PermissionResolved {
                turn_id: 1,
                request_id: Some(request_id),
                ..
            } if request_id == "perm-from-log"
        )));
        assert!(latest_unresolved_permission_request(&events).is_none());
        assert_eq!(
            usecase.turn_phase(&session.id).await,
            Some(TurnPhase::Streaming)
        );

        let loaded = usecase
            .get_session(&session.id)
            .await
            .unwrap()
            .expect("session");
        assert_eq!(loaded.turn_phase, TurnPhase::Streaming);
        assert!(loaded.pending_permission_request.is_none());
        assert!(loaded.pending_permission_state_revision > 0);
    }

    #[tokio::test]
    async fn permission_requested_emits_pending_state_change_when_stream_emit_is_suppressed() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        {
            let mut sessions = usecase.ctx.sessions.lock().await;
            sessions
                .get_mut(&session_id)
                .expect("runtime state")
                .suppress_stream_emit_for_test();
        }

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-suppressed")),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;

        assert!(event_notifier.state_changes().iter().any(|change| {
            change.chat_session_id == session_id
                && change.turn_phase == TurnPhase::WaitingPermission
                && change
                    .pending_permission_request
                    .as_ref()
                    .is_some_and(|request| request.id == "perm-suppressed")
                && change.pending_permission_state_revision.is_some()
        }));
    }

    #[test]
    fn permission_wait_diagnostic_is_marked_once_after_threshold() {
        let mut state = RuntimeSessionState::new("claude".to_string());
        let now = std::time::Instant::now();
        state.install_turn_lease_for_test(TurnPhase::WaitingPermission);
        state.permission_request_cache = Some(permission_request_msg("perm-diag"));
        state.begin_permission_wait_for_test(
            "perm-diag",
            now - PERMISSION_WAIT_DIAGNOSTIC_THRESHOLD - Duration::from_millis(1),
        );

        assert!(maybe_mark_permission_wait_diagnostic("s1", &mut state, now));
        assert!(state.permission_wait_diagnostic_emitted());
        assert!(!maybe_mark_permission_wait_diagnostic(
            "s1", &mut state, now
        ));
    }

    #[test]
    fn permission_wait_diagnostic_skips_fresh_observed_request() {
        let mut state = RuntimeSessionState::new("claude".to_string());
        let now = std::time::Instant::now();
        state.install_turn_lease_for_test(TurnPhase::WaitingPermission);
        state.permission_request_cache = Some(permission_request_msg("perm-visible"));
        state.begin_permission_wait_for_test(
            "perm-visible",
            now - PERMISSION_WAIT_DIAGNOSTIC_THRESHOLD - Duration::from_millis(1),
        );
        state.report_permission_request_observed("perm-visible", true, now);

        assert!(!maybe_mark_permission_wait_diagnostic(
            "s1", &mut state, now
        ));
        assert!(!state.permission_wait_diagnostic_emitted());

        state.report_permission_request_observed(
            "perm-visible",
            true,
            now - PERMISSION_REQUEST_OBSERVED_TTL - Duration::from_millis(1),
        );
        assert!(maybe_mark_permission_wait_diagnostic("s1", &mut state, now));
        assert!(state.permission_wait_diagnostic_emitted());
    }

    #[test]
    fn permission_wait_diagnostic_treats_mismatched_observation_as_unobserved() {
        let mut state = RuntimeSessionState::new("claude".to_string());
        let now = std::time::Instant::now();
        state.install_turn_lease_for_test(TurnPhase::WaitingPermission);
        state.permission_request_cache = Some(permission_request_msg("perm-pending"));
        state.begin_permission_wait_for_test(
            "perm-pending",
            now - PERMISSION_WAIT_DIAGNOSTIC_THRESHOLD - Duration::from_millis(1),
        );
        state.report_permission_request_observed("perm-other", true, now);

        assert!(maybe_mark_permission_wait_diagnostic("s1", &mut state, now));
        assert!(state.permission_wait_diagnostic_emitted());
    }

    #[tokio::test]
    async fn report_permission_request_observed_tracks_matching_pending_request() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store,
                tmp.path(),
            );
        usecase
            .insert_runtime_state_for_test("s1", TurnPhase::WaitingPermission, false)
            .await;
        {
            let mut sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get_mut("s1").expect("runtime state");
            state.set_pending_permission_request(permission_request_msg("perm-visible"));
        }

        usecase
            .report_permission_request_observed("s1", "perm-visible", true)
            .await
            .unwrap();
        {
            let sessions = usecase.ctx.sessions.lock().await;
            assert_eq!(
                sessions
                    .get("s1")
                    .and_then(RuntimeSessionState::visible_permission_request_id),
                Some("perm-visible")
            );
        }

        usecase
            .report_permission_request_observed("s1", "perm-other", false)
            .await
            .unwrap();
        {
            let sessions = usecase.ctx.sessions.lock().await;
            assert_eq!(
                sessions
                    .get("s1")
                    .and_then(RuntimeSessionState::visible_permission_request_id),
                Some("perm-visible")
            );
        }

        usecase
            .report_permission_request_observed("s1", "perm-visible", false)
            .await
            .unwrap();
        {
            let sessions = usecase.ctx.sessions.lock().await;
            assert!(sessions
                .get("s1")
                .and_then(RuntimeSessionState::visible_permission_request_id)
                .is_none());
        }

        usecase
            .report_permission_request_observed("s1", "perm-other", true)
            .await
            .unwrap();
        {
            let sessions = usecase.ctx.sessions.lock().await;
            assert!(sessions
                .get("s1")
                .and_then(RuntimeSessionState::visible_permission_request_id)
                .is_none());
        }
    }

    #[tokio::test]
    async fn skill_catalog_and_mentionable_files_dispatch_to_selected_backend_only() {
        let tmp = tempfile::tempdir().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let usecase = dispatch_test_usecase(tmp.path().to_path_buf(), Arc::clone(&calls), "codex");

        let codex_skills = usecase
            .skill_catalog(Some("codex"), tmp.path(), Some("skill"), Some(5))
            .await
            .unwrap();
        let claude_files = usecase
            .mentionable_files(Some("claude"), tmp.path(), "src", 10)
            .await
            .unwrap()
            .unwrap();
        let default_skills = usecase
            .skill_catalog(None, tmp.path(), None, None)
            .await
            .unwrap();

        assert_eq!(codex_skills[0].scope, "codex");
        assert_eq!(claude_files, vec!["claude-file".to_string()]);
        assert_eq!(default_skills[0].scope, "codex");
        assert_eq!(
            calls.lock().unwrap().clone(),
            vec![
                "codex:skills".to_string(),
                "claude:files".to_string(),
                "codex:skills".to_string()
            ]
        );
    }

    async fn wait_for_call(
        controller: &crate::test_support::TestAgentRuntimeController,
        session_id: &str,
        expected: TestRuntimeCallKind,
    ) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if controller
                    .call_kinds_for(session_id)
                    .iter()
                    .any(|kind| kind == &expected)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_call_count(
        controller: &crate::test_support::TestAgentRuntimeController,
        session_id: &str,
        expected: TestRuntimeCallKind,
        expected_count: usize,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let count = controller
                    .call_kinds_for(session_id)
                    .iter()
                    .filter(|kind| *kind == &expected)
                    .count();
                if count >= expected_count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_stream_delta_count(notifier: &RecordingAgentNotifier, expected_count: usize) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if notifier.streaming_deltas().len() >= expected_count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_error_state_change(notifier: &RecordingAgentNotifier, session_id: &str) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if notifier.state_changes().iter().any(|change| {
                    change.chat_session_id == session_id
                        && change.turn_phase == TurnPhase::Idle
                        && change.session_state == Some(SessionState::Error)
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_stall_observation_count(
        notifier: &RecordingAgentNotifier,
        expected_count: usize,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if notifier.stall_observations().len() >= expected_count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_stall_clear_count(notifier: &RecordingAgentNotifier, expected_count: usize) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if notifier.stall_clears().len() >= expected_count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_workflow_stall_notification_count(
        notifier: &RecordingWorkflowStallNotifier,
        expected_count: usize,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if notifier.notifications().len() >= expected_count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_workflow_stall_cleared_count(
        notifier: &RecordingWorkflowStallNotifier,
        expected_count: usize,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if notifier.cleared_notifications().len() >= expected_count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_stream_emit_failure_state(
        usecase: &AgentSessionRuntimeUsecase,
        session_id: &str,
        predicate: impl Fn(u32, bool) -> bool,
    ) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some((failures, suppressed)) =
                    usecase.stream_emit_failure_state_for_test(session_id).await
                {
                    if predicate(failures, suppressed) {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_last_stream_delta(
        notifier: &RecordingAgentNotifier,
        predicate: impl Fn(&AgentStreamingDeltaPayload) -> bool,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if notifier.streaming_deltas().last().is_some_and(&predicate) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_start_prompt_count(
        controller: &crate::test_support::TestAgentRuntimeController,
        session_id: &str,
        expected_count: usize,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let count = controller
                    .call_kinds_for(session_id)
                    .iter()
                    .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurnPrompt { .. }))
                    .count();
                if count >= expected_count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_open_count(
        controller: &crate::test_support::TestAgentRuntimeController,
        session_id: &str,
        expected_count: usize,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let count = controller
                    .call_kinds_for(session_id)
                    .iter()
                    .filter(|kind| matches!(kind, TestRuntimeCallKind::OpenSession { .. }))
                    .count();
                if count >= expected_count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    fn provider_establish_test_session(
        session_store: &SessionStore,
        data_dir: &Path,
        resume_id: Option<&str>,
    ) -> ChatSession {
        let session = create_session_internal_with_attributes(
            session_store,
            data_dir,
            data_dir.to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        if let Some(resume_id) = resume_id {
            session_store
                .record_backend_session_established(
                    data_dir,
                    &session.id,
                    0,
                    "provider-establish-test-observation",
                    resume_id.to_string(),
                    Some(ContextCarryState::Resumed),
                )
                .unwrap();
        }
        session
    }

    async fn wait_for_turn_phase(
        usecase: &AgentSessionRuntimeUsecase,
        session_id: &str,
        phase: TurnPhase,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if usecase.turn_phase(session_id).await == Some(phase) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    fn persisted_message_parts(
        session_store: &SessionStore,
        data_dir: &Path,
        session_id: &str,
        message_id: &str,
    ) -> Vec<MessagePart> {
        session_store
            .load_full_session_for_restore(data_dir, session_id)
            .unwrap()
            .expect("persisted session")
            .messages
            .into_iter()
            .find(|message| message.id == message_id)
            .and_then(|message| message.parts)
            .unwrap_or_default()
    }

    async fn wait_for_persisted_text(
        session_store: &SessionStore,
        data_dir: &Path,
        session_id: &str,
        message_id: &str,
        expected: &str,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if persisted_message_parts(session_store, data_dir, session_id, message_id)
                    .iter()
                    .any(|part| {
                        matches!(part, MessagePart::Text { content, .. } if content.contains(expected))
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_streaming_text(
        usecase: &AgentSessionRuntimeUsecase,
        session_id: &str,
        expected: &str,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if usecase.streaming_parts(session_id).await.iter().any(|part| {
                    matches!(part, MessagePart::Text { content, .. } if content.contains(expected))
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_session_closing(usecase: &AgentSessionRuntimeUsecase, session_id: &str) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let closing = {
                    let sessions = usecase.ctx.sessions.lock().await;
                    sessions
                        .get(session_id)
                        .is_some_and(RuntimeSessionState::is_closing)
                };
                if closing {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn mark_stall_observation_active_for_test(
        usecase: &AgentSessionRuntimeUsecase,
        session_id: &str,
    ) {
        let mut sessions = usecase.ctx.sessions.lock().await;
        let state = sessions.get_mut(session_id).unwrap();
        state.restore_runtime_progress_for_test(state.last_progress_at(), 1, 0, true);
    }

    #[test]
    fn test_human_parts_画像のみの場合は_image_partを返す() {
        // Given: human input contains no text and one image.
        let parts = human_parts(
            "",
            &[ImageAttachment {
                data: "abc".to_string(),
                media_type: "image/png".to_string(),
            }],
        );

        // Then: the generated human parts preserve the image.
        assert!(matches!(parts[0], MessagePart::Image { .. }));
    }

    #[tokio::test]
    async fn test_send_message_開始成功後に_streaming状態とstatusを通知する() {
        // Given: an agent runtime usecase with recording event/status notifiers.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier.clone(),
        );

        // When: a user sends a message and the backend accepts the turn.
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();

        // Then: the live phase and both notifier surfaces move to Streaming.
        assert_eq!(
            usecase.turn_phase(&response.session.id).await,
            Some(TurnPhase::Streaming)
        );
        assert!(event_notifier.state_changes().iter().any(|change| {
            change.chat_session_id == response.session.id
                && change.turn_phase == TurnPhase::Streaming
                && change.session_state == Some(SessionState::Active)
        }));
        assert!(status_notifier.changes().iter().any(|change| {
            change.session.as_ref().is_some_and(|session| {
                session.chat_session_id == response.session.id
                    && session.turn_phase == TurnPhaseRepr::Streaming
            })
        }));
        assert!(controller
            .call_kinds_for(&response.session.id)
            .contains(&TestRuntimeCallKind::StartTurn));
    }

    #[tokio::test]
    async fn test_send_message_並行送信2本目は二重turnを開始せずqueueへ入る() {
        // Given: an existing session and a runtime whose first start_turn is blocked.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        controller.pause_start_turn();

        // When: two sends race for the same session.
        let first_usecase = Arc::clone(&usecase);
        let first_session_id = session.id.clone();
        let worktree_path = tmp.path().to_string_lossy().to_string();
        let first = tokio::spawn(async move {
            first_usecase
                .send_message(SendAgentMessageRequest {
                    chat_session_id: Some(first_session_id),
                    worktree_path,
                    content: "first".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    backend_id: Some("claude".to_string()),
                    model_id: None,
                    images: None,
                    mentions: None,
                    editor_context: None,
                })
                .await
                .unwrap()
        });
        wait_for_start_prompt_count(&controller, &session.id, 1).await;

        let second_usecase = Arc::clone(&usecase);
        let second_session_id = session.id.clone();
        let worktree_path = tmp.path().to_string_lossy().to_string();
        let second = tokio::spawn(async move {
            second_usecase
                .send_message(SendAgentMessageRequest {
                    chat_session_id: Some(second_session_id),
                    worktree_path,
                    content: "second".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    backend_id: Some("claude".to_string()),
                    model_id: None,
                    images: None,
                    mentions: None,
                    editor_context: None,
                })
                .await
                .unwrap()
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurnPrompt { .. }))
                .count(),
            1
        );
        controller.release_start_turn();
        let first = first.await.unwrap();
        let second = second.await.unwrap();

        // Then: only the first send starts a backend turn; the second is queued.
        assert!(first.agent_message.is_some());
        assert!(first.queued_turn.is_none());
        assert!(second.agent_message.is_none());
        assert!(second.queued_turn.is_some());
        assert_eq!(second.pending_queue_count, 1);
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurnPrompt { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn queued_turn_cancel_is_rejected_without_live_or_restart_visible_mutation() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let first = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = first.session.id;
        wait_for_start_prompt_count(&controller, &session_id, 1).await;
        let queued = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "must remain queued".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        let queued_id = queued.queued_turn.unwrap().id;
        let live_before = usecase.pending_queue(&session_id).await;
        let persisted_before = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .unwrap()
            .messages
            .into_iter()
            .map(|message| (message.id, message.content))
            .collect::<Vec<_>>();
        let events_before = format!(
            "{:?}",
            session_store
                .load_session_events(tmp.path(), &session_id)
                .unwrap()
        );

        let error = usecase
            .cancel_queued_turn(&session_id, Some(&queued_id))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("atomic durable queue operation"));
        assert_eq!(usecase.pending_queue(&session_id).await, live_before);
        assert_eq!(
            format!(
                "{:?}",
                session_store
                    .load_session_events(tmp.path(), &session_id)
                    .unwrap()
            ),
            events_before
        );
        let restarted =
            crate::test_support::build_agent_runtime_usecase(session_store.clone(), tmp.path());
        let persisted_after_restart = restarted
            .get_session(&session_id)
            .await
            .unwrap()
            .unwrap()
            .session
            .messages
            .into_iter()
            .map(|message| (message.id, message.content))
            .collect::<Vec<_>>();
        assert_eq!(persisted_after_restart, persisted_before);
    }

    #[tokio::test]
    async fn test_send_message_queue受理後のprojection障害でも成功応答を返す() {
        // Given: an active turn whose message index does not yet include an orphan chunk, and a
        // projection store that becomes unreadable while the queued human message is persisted.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let first = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = first.session.id;
        wait_for_start_prompt_count(&controller, &session_id, 1).await;
        let orphan = ChatMessage {
            id: "orphan-agent-message".to_string(),
            role: MessageRole::Agent,
            content: "recovered orphan".to_string(),
            thinking: None,
            activities: None,
            parts: None,
            streaming_final_seq: 0,
            timestamp: first.session.updated_at,
            mentions: None,
        };
        let orphan_path = tmp
            .path()
            .join("sessions")
            .join(&session_id)
            .join("messages")
            .join("3.json");
        std::fs::write(
            orphan_path,
            crate::adaptor::gateway::agent_session::session_storage::encode_chat_message_v1(
                &orphan,
            )
            .expect("orphan message must serialize through legacy V1 DTO"),
        )
        .unwrap();
        let titles_path = tmp.path().join("session_titles.json");
        session_store.set_append_message_hook_for_test(Arc::new(move |_, _| {
            std::fs::write(&titles_path, "{").map_err(|error| error.to_string())
        }));

        // When: the follow-up is accepted into the pending queue.
        let response = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "queue exactly once".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .expect("accepted queue input must not fail during response projection");

        // Then: the accepted response uses the append's repaired post-write meta even though a
        // fresh all-session projection now fails, and the queued message exists exactly once.
        assert!(response.queued_turn.is_some());
        assert_eq!(response.pending_queue_count, 1);
        let response_message_count = response
            .sessions
            .iter()
            .find(|summary| summary.id == session_id)
            .map(|summary| summary.message_count)
            .expect("accepted session summary must be present");
        assert!(session_store
            .list_sessions(tmp.path(), tmp.path().to_string_lossy().as_ref())
            .is_err());
        let stored = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .unwrap();
        assert_eq!(response_message_count, 4);
        assert_eq!(response_message_count, stored.messages.len());
        assert!(stored
            .messages
            .iter()
            .any(|message| message.id == orphan.id));
        assert_eq!(
            stored
                .messages
                .iter()
                .filter(|message| message.content == "queue exactly once")
                .count(),
            1
        );
        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
    }

    #[tokio::test]
    async fn test_send_message_queue受理応答のsummaryにcustom_titleを再適用する() {
        // Given: a busy session with an observed stall and a custom title.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let first = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = first.session.id;
        wait_for_start_prompt_count(&controller, &session_id, 1).await;
        mark_stall_observation_active_for_test(&usecase, &session_id).await;
        let custom_title = "Investigate queued follow-up";
        session_store
            .set_session_title(tmp.path(), &session_id, Some(custom_title))
            .unwrap();

        // When: the follow-up is accepted into the pending queue.
        let response = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "queue this".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        // Then: replacing the summary with post-write meta does not discard the custom title.
        assert!(response.queued_turn.is_some());
        assert_eq!(
            response
                .sessions
                .iter()
                .find(|summary| summary.id == session_id)
                .map(|summary| summary.first_message.as_str()),
            Some(custom_title)
        );
    }

    #[tokio::test]
    async fn failed_terminal_pauses_queued_work_until_explicit_resume() {
        // Given: a session whose turn ends as Failed (e.g. Codex remote compact failure).
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let compact_error =
            "Error running remote compact task: stream disconnected before completion".to_string();
        let first = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = first.session.id.clone();
        wait_for_start_prompt_count(&controller, &session_id, 1).await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Error {
                    content: compact_error.clone(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Failed {
                    error: compact_error,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;

        assert!(
            session_store
                .load_queue_paused_at(tmp.path(), &session_id)
                .unwrap()
                .is_some(),
            "a provider failure must durably pause the queue"
        );
        assert!(
            usecase
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused,
            "the live read model must agree with the durable pause"
        );

        // When: the user sends the next message to the same session.
        let second = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "continue".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        // Then: the follow-up remains queued until the user explicitly resumes it.
        assert!(second.agent_message.is_none());
        assert!(second.queued_turn.is_some());
        assert_eq!(second.pending_queue_count, 1);
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .into_iter()
                .filter(|kind| *kind == TestRuntimeCallKind::StartTurn)
                .count(),
            1,
            "failure must not hand the queued input to the provider"
        );

        usecase.resume_queue(&session_id).await.unwrap();
        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );
    }

    async fn enqueue_second_turn_for_test(
        usecase: &Arc<AgentSessionRuntimeUsecase>,
        controller: &crate::test_support::TestAgentRuntimeController,
        worktree_path: String,
    ) -> String {
        let first = usecase
            .send_message(send_request(worktree_path.clone()))
            .await
            .unwrap();
        let session_id = first.session.id.clone();
        wait_for_start_prompt_count(controller, &session_id, 1).await;
        let second = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path,
                content: "queued".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        assert_eq!(second.pending_queue_count, 1);
        session_id
    }

    #[tokio::test]
    async fn damaged_event_log_is_recovered_and_next_message_send_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let first = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = first.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        let event_log_path = tmp
            .path()
            .join("sessions")
            .join(&session_id)
            .join("events.json");
        let content = std::fs::read_to_string(&event_log_path).unwrap();
        let closing_pos = content.rfind(']').expect("event log closing bracket");
        std::fs::write(&event_log_path, &content[..closing_pos]).unwrap();
        take_persistence_log_records();

        let second = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "continue after recovery".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        assert!(second.agent_message.is_some());
        assert!(event_notifier.notices().iter().any(|notice| {
            notice.session_id == session_id && notice.kind == SessionNoticeKind::EventLogRecovered
        }));
        let repaired_events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(repaired_events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::TurnStarted { turn_id: 2, .. })));
        let records = take_persistence_log_records();
        assert!(records.iter().any(|record| matches!(
            record,
            PersistenceLogRecord::EventLogRecovered {
                session_id: logged_session_id,
                kind: "event_log_recovered",
            } if logged_session_id == &session_id
        )));
    }

    #[tokio::test]
    async fn batch_append_reports_event_log_recovery_through_blocking_path() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, _) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let session = crate::usecase::agent_session::session::create_session_internal(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
        )
        .unwrap();
        session_store
            .append_session_event_without_projection(
                tmp.path(),
                &session.id,
                AgentSessionEvent::QueuePaused { at: 1.0 },
            )
            .unwrap();
        let event_log_path = tmp
            .path()
            .join("sessions")
            .join(&session.id)
            .join("events.json");
        let content = std::fs::read_to_string(&event_log_path).unwrap();
        let closing_pos = content.rfind(']').expect("event log closing bracket");
        std::fs::write(&event_log_path, &content[..closing_pos]).unwrap();

        append_session_events_blocking(
            &usecase.ctx,
            &session.id,
            vec![AgentSessionEvent::QueuePaused { at: 2.0 }],
        )
        .await
        .unwrap();

        assert!(event_notifier.notices().iter().any(|notice| {
            notice.session_id == session.id && notice.kind == SessionNoticeKind::EventLogRecovered
        }));
        assert_eq!(
            session_store
                .load_session_events(tmp.path(), &session.id)
                .unwrap()
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::QueuePaused { .. }))
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn reopen_runtime_persist_failure_retries_reports_and_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, _controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session_store.set_state_hook_for_test({
            let attempts = attempts.clone();
            Arc::new(move |_, _| {
                attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err("injected session state failure".to_string())
            })
        });
        take_persistence_log_records();

        let result = persist_with_retry(
            &usecase.ctx,
            &session_id,
            PersistFailureKind::ReopenRuntime,
            || session_store.set_session_state(tmp.path(), &session_id, SessionState::Error),
        )
        .await;

        assert_eq!(
            attempts.load(std::sync::atomic::Ordering::SeqCst),
            PERSIST_MAX_ATTEMPTS
        );
        assert_eq!(result.unwrap_err(), "injected session state failure");
        assert!(event_notifier.notices().iter().any(|notice| {
            notice.session_id == session_id && notice.kind == SessionNoticeKind::PersistFailure
        }));
        assert_eq!(
            usecase
                .ctx
                .status_center
                .get_session(&session_id)
                .and_then(|status| status.notice)
                .map(|notice| notice.kind),
            Some(SessionNoticeKind::PersistFailure)
        );
        let records = take_persistence_log_records();
        assert!(records.iter().any(|record| matches!(
            record,
            PersistenceLogRecord::PersistFailure {
                session_id: logged_session_id,
                kind: "reopen_runtime",
                attempts: PERSIST_MAX_ATTEMPTS,
                error,
            } if logged_session_id == &session_id && error == "injected session state failure"
        )));
    }

    #[tokio::test]
    async fn queued_runtime_reopen_failure_retries_and_stays_visible() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier.clone(),
        );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        usecase
            .prepare_queued_runtime_reopen_for_test(&session_id)
            .await;
        controller.fail_next_open_session();
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session_store.set_state_hook_for_test({
            let attempts = attempts.clone();
            Arc::new(move |_, state| {
                if *state == SessionState::Error {
                    attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    return Err("injected queued reopen state failure".to_string());
                }
                Ok(())
            })
        });
        take_persistence_log_records();

        usecase.drain_next_queued_turn_for_test(&session_id).await;

        assert_eq!(
            attempts.load(std::sync::atomic::Ordering::SeqCst),
            PERSIST_MAX_ATTEMPTS
        );
        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
        assert_eq!(usecase.turn_phase(&session_id).await, Some(TurnPhase::Idle));
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            2
        );
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurnPrompt { .. }))
                .count(),
            1
        );
        assert!(event_notifier.notices().iter().any(|notice| {
            notice.session_id == session_id && notice.kind == SessionNoticeKind::PersistFailure
        }));
        assert!(!event_notifier.state_changes().iter().any(|change| {
            change.chat_session_id == session_id
                && change.turn_phase == TurnPhase::Idle
                && change.session_state == Some(SessionState::Error)
        }));
        let snapshot = usecase
            .ctx
            .status_center
            .get_session(&session_id)
            .expect("status snapshot");
        assert_eq!(snapshot.session_state, SessionState::Active);
        assert_eq!(
            snapshot.notice.map(|notice| notice.kind),
            Some(SessionNoticeKind::PersistFailure)
        );
        assert!(status_notifier.changes().iter().any(|changes| {
            changes.session.as_ref().is_some_and(|status| {
                status.chat_session_id == session_id
                    && status
                        .notice
                        .as_ref()
                        .is_some_and(|notice| notice.kind == SessionNoticeKind::PersistFailure)
            })
        }));
        assert_eq!(
            session_store
                .get_session_shell(tmp.path(), &session_id)
                .unwrap()
                .expect("durable session")
                .state,
            SessionState::Active
        );
        let records = take_persistence_log_records();
        assert!(records.iter().any(|record| matches!(
            record,
            PersistenceLogRecord::PersistFailure {
                session_id: logged_session_id,
                kind: "reopen_runtime",
                attempts: PERSIST_MAX_ATTEMPTS,
                error,
            } if logged_session_id == &session_id
                && error == "injected queued reopen state failure"
        )));
    }

    #[tokio::test]
    async fn transient_persist_failure_recovers_without_notice() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, _controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session_store.set_state_hook_for_test({
            let attempts = attempts.clone();
            Arc::new(move |_, _| {
                if attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    Err("transient session state failure".to_string())
                } else {
                    Ok(())
                }
            })
        });

        persist_with_retry(
            &usecase.ctx,
            &session_id,
            PersistFailureKind::ReopenRuntime,
            || session_store.set_session_state(tmp.path(), &session_id, SessionState::Error),
        )
        .await
        .unwrap();

        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(event_notifier.notices().is_empty());
    }

    #[tokio::test]
    async fn successful_persist_clears_previous_failure_notice() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, _controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier,
            status_notifier.clone(),
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        session_store.set_state_hook_for_test(Arc::new(|_, _| {
            Err("injected session state failure".to_string())
        }));
        persist_with_retry(
            &usecase.ctx,
            &session_id,
            PersistFailureKind::ReopenRuntime,
            || session_store.set_session_state(tmp.path(), &session_id, SessionState::Error),
        )
        .await
        .unwrap_err();
        assert!(usecase
            .ctx
            .status_center
            .get_session(&session_id)
            .and_then(|status| status.notice)
            .is_some());

        session_store.set_state_hook_for_test(Arc::new(|_, _| Ok(())));
        persist_with_retry(
            &usecase.ctx,
            &session_id,
            PersistFailureKind::ReopenRuntime,
            || session_store.set_session_state(tmp.path(), &session_id, SessionState::Error),
        )
        .await
        .unwrap();

        assert!(usecase
            .ctx
            .status_center
            .get_session(&session_id)
            .and_then(|status| status.notice)
            .is_none());
        assert!(status_notifier.changes().iter().any(|changes| {
            changes.session.as_ref().is_some_and(|status| {
                status.chat_session_id == session_id && status.notice.is_none()
            })
        }));
    }

    #[tokio::test]
    async fn projection_retry_does_not_append_event_twice_after_partial_success() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, _controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier,
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let state_attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session_store.set_state_hook_for_test({
            let state_attempts = state_attempts.clone();
            Arc::new(move |_, _| {
                state_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err("injected post-append projection failure".to_string())
            })
        });
        let event = AgentSessionEvent::FinalPartsRecorded {
            turn_id: 1,
            message_id: "agent-message".to_string(),
            parts: vec![MessagePart::Text {
                content: "durable once".to_string(),
                parent_tool_use_id: None,
            }],
        };

        let result = append_session_event_and_project_state_with_retry(
            &usecase.ctx,
            &session_id,
            PersistFailureKind::FinalPartsRecorded,
            event.clone(),
        )
        .await;

        assert_eq!(
            result.unwrap_err(),
            "injected post-append projection failure"
        );
        assert_eq!(
            state_attempts.load(std::sync::atomic::Ordering::SeqCst),
            PERSIST_MAX_ATTEMPTS
        );
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|candidate| **candidate == event)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn b024_normal_completion_commits_one_terminal_then_drains_the_next_queue_item() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();

        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        assert!(usecase.pending_queue(&session_id).await.is_empty());
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::TurnCompleted { turn_id: 1, .. }
                ))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::TurnStarted { .. }))
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn b040_unresolved_recovery_blocks_automatic_queue_drain_without_provider_effect() {
        let tmp = tempfile::tempdir().unwrap();
        let local_store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
        session_store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        let recovery_id = "recovery-blocks-queue-drain";
        local_store
            .commit_batch(LocalAtomicBatch {
                commit_id: CommitIdentity::parse("b040-blocker-commit").unwrap(),
                idempotency: IdempotencyBinding {
                    installation_id: local_store.installation_id().to_string(),
                    operation_kind: CommitOperationKind::Recovery,
                    idempotency_key: "b040-blocker".to_string(),
                    payload_hash: [40; 32],
                },
                expected_heads: Vec::new(),
                events: Vec::new(),
                state_mutations: vec![LocalStateMutation::Obligation(ObligationMutation {
                    obligation_id: recovery_id.to_string(),
					record: crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
						session_id: session_id.clone(),
						recovery_id: recovery_id.to_string(),
						detail: crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
							old_provider_session_generation: 0,
							reason: crate::domain::agent_session::events::BackendSessionRecoveryReason::BackendSessionLost,
							reserved_at_bits: 0,
						},
						state: crate::domain::local_event::ObligationStateRecord::ReconciliationRequired,
					},
                    pending: Some(PendingIndexEntry {
                        ordered_key: format!("{recovery_id}:0001"),
                        owner: session_id.clone(),
                        partition: PendingPartition::Owner,
                        shutdown_plan: None,
                    }),
                    expected: RevisionGuard::Absent,
                    revision: Revision::new(0).unwrap(),
                })],
            })
            .await
            .unwrap();

        let failure = session_store
            .ensure_no_unresolved_recovery(&session_id)
            .await
            .unwrap_err();
        assert_eq!(failure.correlation_id, recovery_id);
        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurnPrompt { .. }))
                .count(),
            1,
            "the unresolved recovery fence must run before queue/provider dispatch"
        );
    }

    #[tokio::test]
    async fn queued_turn_append_message_failure_preserves_queue_and_retries() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        let fail_once = Arc::new(std::sync::atomic::AtomicBool::new(true));
        session_store.set_append_message_hook_for_test({
            let fail_once = Arc::clone(&fail_once);
            Arc::new(move |_, message| {
                if message.role == MessageRole::Agent
                    && fail_once.swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    return Err("injected append message failure".to_string());
                }
                Ok(())
            })
        });

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;

        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurnPrompt { .. }))
                .count(),
            1
        );
        usecase.drain_next_queued_turn_for_test(&session_id).await;
        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        assert!(usecase.pending_queue(&session_id).await.is_empty());
    }

    #[tokio::test]
    async fn queued_turn_started_event_failure_preserves_queue_and_retries() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        let fail_once = Arc::new(std::sync::atomic::AtomicBool::new(true));
        session_store.set_append_event_hook_for_test({
            let fail_once = Arc::clone(&fail_once);
            Arc::new(move |_, event| {
                if matches!(event, AgentSessionEvent::TurnStarted { .. })
                    && fail_once.swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    return Err("injected turn started failure".to_string());
                }
                Ok(())
            })
        });

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;

        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
        usecase.drain_next_queued_turn_for_test(&session_id).await;
        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        assert!(usecase.pending_queue(&session_id).await.is_empty());
    }

    #[tokio::test]
    async fn queued_turn_start_turn_failure_preserves_queue_and_retries() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        controller.fail_next_start_turn();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;

        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session_id)
            .unwrap()
            .is_some());
        usecase.resume_queue(&session_id).await.unwrap();
        wait_for_start_prompt_count(&controller, &session_id, 3).await;
        assert!(usecase.pending_queue(&session_id).await.is_empty());
    }

    #[tokio::test]
    async fn queued_turn_interrupt_append_retries_then_reports() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session_store.set_append_event_hook_for_test({
            let attempts = attempts.clone();
            Arc::new(move |_, event| {
                if matches!(event, AgentSessionEvent::TurnInterrupted { .. }) {
                    attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    return Err("injected turn interruption failure".to_string());
                }
                Ok(())
            })
        });
        controller.fail_next_start_turn();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if attempts.load(std::sync::atomic::Ordering::SeqCst) == PERSIST_MAX_ATTEMPTS
                    && event_notifier.notices().iter().any(|notice| {
                        notice.session_id == session_id
                            && notice.kind == SessionNoticeKind::PersistFailure
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("queued interruption persistence should exhaust retries");

        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
    }

    #[tokio::test]
    async fn turn_completed_append_failure_is_retained_until_the_terminal_commit_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier.clone(),
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session_store.set_append_event_hook_for_test({
            let attempts = attempts.clone();
            Arc::new(move |_, event| {
                if matches!(event, AgentSessionEvent::TurnCompleted { .. }) {
                    let attempt = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if attempt < PERSIST_MAX_ATTEMPTS {
                        return Err("injected turn completed failure".to_string());
                    }
                }
                Ok(())
            })
        });

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if usecase.turn_phase(&session_id).await == Some(TurnPhase::Idle) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the retained terminal event should commit after storage recovers");
        assert_eq!(usecase.turn_phase(&session_id).await, Some(TurnPhase::Idle));

        assert_eq!(
            attempts.load(std::sync::atomic::Ordering::SeqCst),
            PERSIST_MAX_ATTEMPTS + 1
        );
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::FinalPartsRecorded { .. })));
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::TurnCompleted { .. })));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::TurnCompleted { .. }))
                .count(),
            1
        );
        assert!(event_notifier.notices().iter().any(|notice| {
            notice.session_id == session_id && notice.kind == SessionNoticeKind::PersistFailure
        }));
        assert!(status_notifier.changes().iter().any(|changes| {
            changes.session.as_ref().is_some_and(|status| {
                status.chat_session_id == session_id
                    && status
                        .notice
                        .as_ref()
                        .is_some_and(|notice| notice.kind == SessionNoticeKind::PersistFailure)
            })
        }));
    }

    #[tokio::test]
    async fn final_parts_append_failure_keeps_body_not_tool_only() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![
                    DomainMessagePart::Text {
                        content: "persisted response body".to_string(),
                        parent_tool_use_id: None,
                    },
                    DomainMessagePart::ToolUse {
                        id: "tool-1".to_string(),
                        tool: "Bash".to_string(),
                        input:
                            crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                                "{}".to_string(),
                            ),
                        parent_tool_use_id: None,
                    },
                ]),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let parts = usecase.streaming_parts(&session_id).await;
                if parts
                    .iter()
                    .any(|part| matches!(part, MessagePart::Text { .. }))
                    && parts
                        .iter()
                        .any(|part| matches!(part, MessagePart::ToolUse { .. }))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("streaming body and tool part should be applied");
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session_store.set_append_event_hook_for_test({
            let attempts = attempts.clone();
            Arc::new(move |_, event| {
                if matches!(event, AgentSessionEvent::FinalPartsRecorded { .. }) {
                    attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    return Err("injected final parts failure".to_string());
                }
                Ok(())
            })
        });

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if attempts.load(std::sync::atomic::Ordering::SeqCst) == PERSIST_MAX_ATTEMPTS
                    && event_notifier.notices().iter().any(|notice| {
                        notice.session_id == session_id
                            && notice.kind == SessionNoticeKind::PersistFailure
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("final parts persistence should exhaust retries");
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );

        let fresh_store = build_session_store();
        let reloaded = fresh_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("reloaded session");
        let agent_parts = reloaded
            .messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::Agent)
            .and_then(|message| message.parts.as_ref())
            .expect("agent message parts");
        assert!(agent_parts.iter().any(|part| matches!(
            part,
            MessagePart::Text { content, .. } if content == "persisted response body"
        )));
        assert!(agent_parts
            .iter()
            .any(|part| matches!(part, MessagePart::ToolUse { .. })));
    }

    #[tokio::test]
    async fn interrupt_durably_pauses_queue_and_resume_explicitly_starts_it() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        let appended = Arc::new(Mutex::new(Vec::new()));
        let appended_for_hook = Arc::clone(&appended);
        let controller_for_hook = controller.clone();
        session_store.set_append_event_hook_for_test(Arc::new(move |session_id, event| {
            if matches!(
                event,
                AgentSessionEvent::TurnInterruptRequested { .. }
                    | AgentSessionEvent::QueuePaused { .. }
            ) {
                assert!(!controller_for_hook
                    .call_kinds_for(session_id)
                    .contains(&TestRuntimeCallKind::Interrupt));
                appended_for_hook.lock().unwrap().push(event.clone());
            }
            Ok(())
        }));

        usecase.interrupt(&session_id).await.unwrap();

        {
            let appended = appended.lock().unwrap();
            assert_eq!(appended.len(), 2);
            assert!(matches!(
                &appended[0],
                AgentSessionEvent::TurnInterruptRequested { turn_id: 1, .. }
            ));
            assert!(matches!(
                &appended[1],
                AgentSessionEvent::QueuePaused { .. }
            ));
        }
        assert!(controller
            .call_kinds_for(&session_id)
            .contains(&TestRuntimeCallKind::Interrupt));
        assert!(
            usecase
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                    reason: DomainInterruptReason::Abort,
                    error: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurn))
                .count(),
            1
        );

        usecase.resume_queue(&session_id).await.unwrap();

        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if usecase.pending_queue(&session_id).await.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            !usecase
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );
    }

    #[tokio::test]
    async fn resume_append_failure_keeps_the_pending_queue_durably_paused() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        usecase.interrupt(&session_id).await.unwrap();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                    reason: DomainInterruptReason::Abort,
                    error: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        let start_count_before_resume = controller
            .call_kinds_for(&session_id)
            .iter()
            .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurn))
            .count();
        let state_change_count_before_resume = event_notifier.state_changes().len();
        session_store.set_append_event_hook_for_test(Arc::new(|_, event| {
            if matches!(event, AgentSessionEvent::QueueResumed { .. }) {
                return Err("injected QueueResumed append failure".to_string());
            }
            Ok(())
        }));

        let error = usecase.resume_queue(&session_id).await.unwrap_err();

        assert!(error
            .to_string()
            .contains("injected QueueResumed append failure"));
        assert!(
            usecase
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );
        assert_eq!(
            event_notifier.state_changes().len(),
            state_change_count_before_resume
        );
        assert!(!event_notifier
            .state_changes()
            .iter()
            .skip(state_change_count_before_resume)
            .any(|change| change.queue_paused == Some(false)));
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurn))
                .count(),
            start_count_before_resume
        );
        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session_id)
            .unwrap()
            .is_some());

        let restarted =
            crate::test_support::build_agent_runtime_usecase(session_store.clone(), tmp.path());
        assert!(
            restarted
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );
    }

    #[tokio::test]
    async fn shutdown_admission_rejects_queue_resume_without_clearing_the_durable_pause() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        usecase.interrupt(&session_id).await.unwrap();
        usecase.ctx.shutdown_admission.begin_shutdown();

        let error = usecase.resume_queue(&session_id).await.unwrap_err();

        assert!(error.to_string().contains("shutting down"));
        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session_id)
            .unwrap()
            .is_some());
        assert!(!session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::QueueResumed { .. })));
    }

    #[tokio::test]
    async fn interrupt_after_active_turn_resume_reestablishes_the_durable_pause() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;

        usecase.interrupt(&session_id).await.unwrap();
        usecase.resume_queue(&session_id).await.unwrap();
        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session_id)
            .unwrap()
            .is_none());
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );

        usecase.interrupt(&session_id).await.unwrap();

        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session_id)
            .unwrap()
            .is_some());
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                    reason: DomainInterruptReason::Abort,
                    error: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurn))
                .count(),
            1
        );
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::QueuePaused { .. }))
                .count(),
            2
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::QueueResumed { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn concurrent_resume_is_persisted_after_the_inflight_pause() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let gate = Arc::new((Mutex::new((false, false)), Condvar::new()));
        let hook_gate = Arc::clone(&gate);
        session_store.set_append_event_hook_for_test(Arc::new(move |_, event| {
            if matches!(event, AgentSessionEvent::QueuePaused { .. }) {
                let (lock, condvar) = &*hook_gate;
                let mut state = lock.lock().unwrap();
                state.0 = true;
                condvar.notify_all();
                while !state.1 {
                    state = condvar.wait(state).unwrap();
                }
            }
            Ok(())
        }));

        let interrupt_usecase = Arc::clone(&usecase);
        let interrupt_session_id = session_id.clone();
        let interrupt = tokio::spawn(async move {
            interrupt_usecase
                .interrupt(&interrupt_session_id)
                .await
                .unwrap();
        });
        loop {
            if gate.0.lock().unwrap().0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        let resume_usecase = Arc::clone(&usecase);
        let resume_session_id = session_id.clone();
        let resume = tokio::spawn(async move {
            resume_usecase
                .resume_queue(&resume_session_id)
                .await
                .unwrap();
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!resume.is_finished());

        {
            let (lock, condvar) = &*gate;
            let mut state = lock.lock().unwrap();
            state.1 = true;
            condvar.notify_all();
        }
        interrupt.await.unwrap();
        resume.await.unwrap();

        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        let pause_index = events
            .iter()
            .position(|event| matches!(event, AgentSessionEvent::QueuePaused { .. }))
            .unwrap();
        let resume_index = events
            .iter()
            .position(|event| matches!(event, AgentSessionEvent::QueueResumed { .. }))
            .unwrap();
        assert!(pause_index < resume_index);
        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session_id)
            .unwrap()
            .is_none());
        assert!(
            !usecase
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );
        assert!(controller
            .call_kinds_for(&session_id)
            .contains(&TestRuntimeCallKind::Interrupt));
    }

    #[tokio::test]
    async fn interrupt_watchdog_force_finalizes_an_unresponsive_backend_as_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let generation = {
            let sessions = usecase.ctx.sessions.lock().await;
            sessions.get(&session_id).unwrap().generation()
        };

        usecase.interrupt(&session_id).await.unwrap();
        spawn_interrupt_watchdog_task(
            &usecase.ctx,
            session_id.clone(),
            generation,
            Duration::from_millis(10),
        );
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;

        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::Timeout,
                ..
            }
        )));
        let loaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(loaded.session.state, SessionState::Error);
        assert!(loaded.queue_paused);
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::Interrupt))
                .count(),
            1
        );
    }

    #[tokio::test(start_paused = true)]
    async fn production_interrupt_watchdog_finalizes_at_the_ten_second_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let mut request = send_request(tmp.path().to_string_lossy().to_string());
        request.backend_id = Some("codex".to_string());
        let response = usecase.send_message(request).await.unwrap();
        let session_id = response.session.id;
        let queued = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "queued input stays intact".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: Some(vec![ImageAttachment {
                    data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=".to_string(),
                    media_type: "image/png".to_string(),
                }]),
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        assert_eq!(queued.pending_queue_count, 1);
        assert_eq!(
            queued.pending_queue[0].content_preview,
            "queued input stays intact"
        );
        assert_eq!(queued.pending_queue[0].image_count, 1);

        usecase.interrupt(&session_id).await.unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(9)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );

        tokio::time::advance(Duration::from_secs(1)).await;
        for _ in 0..20 {
            if usecase.turn_phase(&session_id).await == Some(TurnPhase::Idle) {
                break;
            }
            tokio::task::yield_now().await;
        }

        assert_eq!(usecase.turn_phase(&session_id).await, Some(TurnPhase::Idle));
        let preserved_queue = usecase.pending_queue(&session_id).await;
        assert_eq!(preserved_queue.len(), 1);
        assert_eq!(
            preserved_queue[0].content_preview,
            "queued input stays intact"
        );
        assert_eq!(preserved_queue[0].image_count, 1);
        assert!(
            usecase
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );
        assert!(controller
            .call_kinds_for(&session_id)
            .contains(&TestRuntimeCallKind::Close));
        assert!(session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::TurnInterrupted {
                    reason: EventInterruptReason::Timeout,
                    ..
                }
            )));
    }

    #[tokio::test(start_paused = true)]
    async fn production_interrupt_from_waiting_permission_clears_permission_and_stays_paused() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-stop")),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;

        usecase.interrupt(&session_id).await.unwrap();

        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session_id)
            .unwrap()
            .is_some());
        tokio::task::yield_now().await;
        tokio::time::advance(INTERRUPT_FORCE_FINALIZE_DELAY).await;
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        let loaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert!(loaded.queue_paused);
        assert!(loaded.pending_permission_request.is_none());
        assert!(session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::TurnInterrupted {
                    reason: EventInterruptReason::Timeout,
                    exit_code: 124,
                    ..
                }
            )));
    }

    #[tokio::test(start_paused = true)]
    async fn backend_interrupt_failure_keeps_the_accepted_stop_until_timeout_terminal() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller.fail_next_interrupt();

        usecase.interrupt(&session_id).await.unwrap();

        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session_id)
            .unwrap()
            .is_some());
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::Interrupt))
                .count(),
            1
        );
        tokio::task::yield_now().await;
        tokio::time::advance(INTERRUPT_FORCE_FINALIZE_DELAY).await;
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        let loaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert!(loaded.queue_paused);
        assert!(session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::TurnInterrupted {
                    reason: EventInterruptReason::Timeout,
                    exit_code: 124,
                    ..
                }
            )));
    }

    #[tokio::test(start_paused = true)]
    async fn claude_synthetic_timeout_wins_the_timer_race_without_changing_the_terminal_reason() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let synthetic_controller = controller.clone();
        let synthetic_session_id = session_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(INTERRUPT_FORCE_FINALIZE_DELAY).await;
            synthetic_controller
                .emit(
                    &synthetic_session_id,
                    AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                        reason: DomainInterruptReason::Timeout,
                        error: None,
                    }),
                )
                .unwrap();
        });
        tokio::task::yield_now().await;

        usecase.interrupt(&session_id).await.unwrap();
        tokio::time::advance(INTERRUPT_FORCE_FINALIZE_DELAY).await;
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        tokio::task::yield_now().await;

        let terminal_events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap()
            .into_iter()
            .filter(|event| {
                matches!(
                    event,
                    AgentSessionEvent::TurnCompleted { .. }
                        | AgentSessionEvent::TurnInterrupted { .. }
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(terminal_events.len(), 1);
        assert!(matches!(
            terminal_events[0],
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::Timeout,
                exit_code: 124,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn provider_terminal_results_after_interrupt_are_preserved() {
        for result in [
            TurnResult::Completed {
                stop_reason: None,
                token_usage: None,
            },
            TurnResult::Failed {
                error: "late start failure".to_string(),
                token_usage: None,
            },
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let session_store = Arc::new(build_session_store());
            let (usecase, controller) =
                crate::test_support::build_agent_runtime_usecase_with_controller(
                    session_store.clone(),
                    tmp.path(),
                );
            let response = usecase
                .send_message(send_request(tmp.path().to_string_lossy().to_string()))
                .await
                .unwrap();
            let session_id = response.session.id;

            usecase.interrupt(&session_id).await.unwrap();
            controller
                .emit(&session_id, AgentRuntimeEvent::TurnCompleted(result))
                .unwrap();
            wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;

            let events = session_store
                .load_session_events(tmp.path(), &session_id)
                .unwrap();
            assert!(events
                .iter()
                .any(|event| matches!(event, AgentSessionEvent::TurnCompleted { .. })));
            assert!(!events.iter().any(|event| matches!(
                event,
                AgentSessionEvent::TurnInterrupted {
                    reason: EventInterruptReason::Abort,
                    ..
                }
            )));
        }
    }

    #[tokio::test]
    async fn queue_pause_and_explicit_resume_survive_runtime_state_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        usecase.interrupt(&session_id).await.unwrap();

        let restarted =
            crate::test_support::build_agent_runtime_usecase(session_store.clone(), tmp.path());
        assert!(
            restarted
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );

        restarted.resume_queue(&session_id).await.unwrap();
        let restarted_again =
            crate::test_support::build_agent_runtime_usecase(session_store.clone(), tmp.path());
        assert!(
            !restarted_again
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );
        assert!(session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::QueueResumed { .. })));
    }

    #[tokio::test]
    async fn queue_resume_after_restart_is_fenced_by_unfinished_backend_recovery() {
        let tmp = tempfile::tempdir().unwrap();
        let original_store = Arc::new(build_session_store());
        let session = create_session_internal_with_attributes(
            &original_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        original_store
            .begin_backend_session_recovery(
                tmp.path(),
                &session.id,
                "resume-fence-recovery",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        original_store
            .append_session_events(
                tmp.path(),
                &session.id,
                &[AgentSessionEvent::QueuePaused { at: 8.0 }],
            )
            .unwrap();
        drop(original_store);
        let reopened_store = Arc::new(build_session_store());
        let (restarted, controller) =
            build_agent_runtime_usecase_with_controller(reopened_store.clone(), tmp.path());

        let error = restarted.resume_queue(&session.id).await.unwrap_err();

        assert!(
            error.to_string().contains("requires reconciliation"),
            "unexpected recovery fence error: {error}"
        );
        assert!(reopened_store
            .load_queue_paused_at(tmp.path(), &session.id)
            .unwrap()
            .is_some());
        assert!(!reopened_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::QueueResumed { .. })));
        assert!(!controller
            .call_kinds_for(&session.id)
            .iter()
            .any(|kind| matches!(kind, TestRuntimeCallKind::StartTurn)));
    }

    #[tokio::test]
    async fn durable_pause_is_hydrated_before_direct_send_after_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let mut first_request = send_request(tmp.path().to_string_lossy().to_string());
        first_request.backend_id = Some("codex".to_string());
        let response = usecase.send_message(first_request).await.unwrap();
        let session_id = response.session.id;
        usecase.interrupt(&session_id).await.unwrap();

        let (restarted, restarted_controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store,
                tmp.path(),
            );
        let queued = restarted
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "must remain queued until explicit resume".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        assert_eq!(queued.pending_queue_count, 1);
        assert!(!restarted_controller
            .call_kinds_for(&session_id)
            .iter()
            .any(|kind| matches!(kind, TestRuntimeCallKind::StartTurn)));

        restarted.resume_queue(&session_id).await.unwrap();
        wait_for_start_prompt_count(&restarted_controller, &session_id, 1).await;
    }

    #[tokio::test]
    async fn interrupt_while_runtime_open_is_pending_prevents_provider_turn_start() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store,
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &usecase.ctx.session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        controller.pause_open_session();
        let send_usecase = Arc::clone(&usecase);
        let session_id = session.id.clone();
        let worktree_path = tmp.path().to_string_lossy().to_string();
        let send = tokio::spawn(async move {
            send_usecase
                .send_message(SendAgentMessageRequest {
                    chat_session_id: Some(session_id),
                    worktree_path,
                    content: "stop during runtime open".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    backend_id: Some("codex".to_string()),
                    model_id: None,
                    images: None,
                    mentions: None,
                    editor_context: None,
                })
                .await
        });
        for _ in 0..100 {
            if controller
                .call_kinds_for(&session.id)
                .iter()
                .any(|kind| matches!(kind, TestRuntimeCallKind::OpenSession { .. }))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(controller
            .call_kinds_for(&session.id)
            .iter()
            .any(|kind| matches!(kind, TestRuntimeCallKind::OpenSession { .. })));
        let generation = {
            let sessions = usecase.ctx.sessions.lock().await;
            sessions.get(&session.id).unwrap().generation()
        };

        usecase.interrupt(&session.id).await.unwrap();
        controller.release_open_session();
        send.await.unwrap().unwrap();

        assert!(!controller
            .call_kinds_for(&session.id)
            .iter()
            .any(|kind| matches!(kind, TestRuntimeCallKind::StartTurn)));
        force_finalize_interrupted_turn(&usecase.ctx, &session.id, generation).await;
        assert_eq!(usecase.turn_phase(&session.id).await, Some(TurnPhase::Idle));
    }

    #[tokio::test(start_paused = true)]
    async fn runtime_open_failure_after_interrupt_timeout_does_not_replace_the_terminal_result() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let session = create_session_internal_with_attributes(
            &usecase.ctx.session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        controller.pause_open_session();
        controller.fail_next_open_session();
        let send_usecase = Arc::clone(&usecase);
        let session_id = session.id.clone();
        let worktree_path = tmp.path().to_string_lossy().to_string();
        let send = tokio::spawn(async move {
            send_usecase
                .send_message(SendAgentMessageRequest {
                    chat_session_id: Some(session_id),
                    worktree_path,
                    content: "runtime open eventually fails".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    backend_id: Some("codex".to_string()),
                    model_id: None,
                    images: None,
                    mentions: None,
                    editor_context: None,
                })
                .await
        });
        for _ in 0..100 {
            if controller
                .call_kinds_for(&session.id)
                .iter()
                .any(|kind| matches!(kind, TestRuntimeCallKind::OpenSession { .. }))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(controller
            .call_kinds_for(&session.id)
            .iter()
            .any(|kind| matches!(kind, TestRuntimeCallKind::OpenSession { .. })));

        usecase.interrupt(&session.id).await.unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(INTERRUPT_FORCE_FINALIZE_DELAY).await;
        for _ in 0..20 {
            if usecase.turn_phase(&session.id).await == Some(TurnPhase::Idle) {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(usecase.turn_phase(&session.id).await, Some(TurnPhase::Idle));
        let terminal_notification_count = event_notifier.state_changes().len();

        controller.release_open_session();
        send.await
            .unwrap()
            .expect("timeout terminal result owns the late runtime open failure");
        tokio::task::yield_now().await;

        assert_eq!(
            event_notifier.state_changes().len(),
            terminal_notification_count,
            "late runtime open failure must not publish a second error state"
        );
        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::TurnCompleted { .. }
                        | AgentSessionEvent::TurnInterrupted { .. }
                ))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::TurnInterrupted {
                        reason: EventInterruptReason::Timeout,
                        ..
                    }
                ))
                .count(),
            1
        );
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::Crash,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn runtime_state_hydration_rejects_a_missing_backend_without_claude_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let usecase = crate::test_support::build_agent_runtime_usecase(session_store, tmp.path());
        let mut session = create_session_internal_with_attributes(
            &usecase.ctx.session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes::default(),
        )
        .unwrap();
        session.backend_id = None;

        let error = usecase
            .hydrate_runtime_session_state(&session)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("missing backend id"));
        assert!(!usecase.ctx.sessions.lock().await.contains_key(&session.id));
    }

    #[tokio::test]
    async fn b027_past_turn_late_streaming_and_terminal_events_leave_new_turn_unchanged_live_and_reload(
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store,
                tmp.path(),
            );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        let generation = {
            let sessions = usecase.ctx.sessions.lock().await;
            sessions.get(&session_id).unwrap().generation()
        };
        usecase.interrupt(&session_id).await.unwrap();
        force_finalize_interrupted_turn(&usecase.ctx, &session_id, generation).await;

        usecase.resume_queue(&session_id).await.unwrap();
        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            2
        );
        let before_session = usecase.get_session(&session_id).await.unwrap().unwrap();
        let before_events = usecase
            .ctx
            .session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        let project_messages = |messages: &[ChatMessage]| {
            messages
                .iter()
                .map(|message| {
                    (
                        message.id.clone(),
                        message.role.clone(),
                        message.content.clone(),
                        message.parts.clone(),
                        message.streaming_final_seq,
                    )
                })
                .collect::<Vec<_>>()
        };
        let before_messages = project_messages(&before_session.session.messages);
        let before_state = before_session.session.state;
        controller
            .emit_for_runtime(
                &session_id,
                0,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "late t-1 output".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        controller
            .emit_for_runtime(
                &session_id,
                0,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );
        let after_live = usecase.get_session(&session_id).await.unwrap().unwrap();
        let after_reload = usecase
            .ctx
            .session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .unwrap();
        let after_events = usecase
            .ctx
            .session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            project_messages(&after_live.session.messages),
            before_messages
        );
        assert_eq!(project_messages(&after_reload.messages), before_messages);
        assert_eq!(after_live.session.state, before_state);
        assert_eq!(after_reload.state, before_state);
        assert_eq!(after_events, before_events);
        assert!(!after_live.session.messages.iter().any(|message| {
            message.parts.as_deref().is_some_and(|parts| {
                parts.iter().any(|part| {
                    matches!(part, MessagePart::Text { content, .. } if content == "late t-1 output")
                })
            })
        }));
    }

    #[tokio::test]
    async fn timeout_while_start_turn_is_pending_does_not_publish_late_streaming_state() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let session = create_session_internal_with_attributes(
            &usecase.ctx.session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        controller.pause_start_turn();
        let send_usecase = Arc::clone(&usecase);
        let session_id = session.id.clone();
        let worktree_path = tmp.path().to_string_lossy().to_string();
        let send = tokio::spawn(async move {
            send_usecase
                .send_message(SendAgentMessageRequest {
                    chat_session_id: Some(session_id),
                    worktree_path,
                    content: "start then stop".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    backend_id: Some("codex".to_string()),
                    model_id: None,
                    images: None,
                    mentions: None,
                    editor_context: None,
                })
                .await
        });
        wait_for_start_prompt_count(&controller, &session.id, 1).await;
        let generation = {
            let sessions = usecase.ctx.sessions.lock().await;
            sessions.get(&session.id).unwrap().generation()
        };

        usecase.interrupt(&session.id).await.unwrap();
        force_finalize_interrupted_turn(&usecase.ctx, &session.id, generation).await;
        controller.release_start_turn();
        send.await.unwrap().unwrap();
        tokio::task::yield_now().await;

        assert_eq!(usecase.turn_phase(&session.id).await, Some(TurnPhase::Idle));
        let changes = event_notifier.state_changes();
        let terminal_index = changes
            .iter()
            .rposition(|change| {
                change.chat_session_id == session.id && change.turn_phase == TurnPhase::Idle
            })
            .expect("timeout terminal notification");
        assert!(!changes.iter().skip(terminal_index + 1).any(|change| {
            change.chat_session_id == session.id && change.turn_phase == TurnPhase::Streaming
        }));
    }

    #[tokio::test(start_paused = true)]
    async fn interrupt_timeout_releases_command_waiters_while_old_start_turn_remains_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store,
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &usecase.ctx.session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        let first_start_gate = controller.pause_next_start_turn();
        let first = {
            let usecase = Arc::clone(&usecase);
            let session_id = session.id.clone();
            let worktree_path = tmp.path().to_string_lossy().to_string();
            tokio::spawn(async move {
                usecase
                    .send_message(SendAgentMessageRequest {
                        chat_session_id: Some(session_id),
                        worktree_path,
                        content: "provider start remains pending".to_string(),
                        permission_mode: PermissionMode::Edit,
                        plan_mode: false,
                        backend_id: Some("codex".to_string()),
                        model_id: None,
                        images: None,
                        mentions: None,
                        editor_context: None,
                    })
                    .await
            })
        };
        for _ in 0..100 {
            if controller
                .call_kinds_for(&session.id)
                .iter()
                .any(|kind| matches!(kind, TestRuntimeCallKind::StartTurn))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(!first.is_finished());

        let waiting_send = {
            let usecase = Arc::clone(&usecase);
            let session_id = session.id.clone();
            let worktree_path = tmp.path().to_string_lossy().to_string();
            tokio::spawn(async move {
                usecase
                    .send_message(SendAgentMessageRequest {
                        chat_session_id: Some(session_id),
                        worktree_path,
                        content: "queue after timeout".to_string(),
                        permission_mode: PermissionMode::Edit,
                        plan_mode: false,
                        backend_id: Some("codex".to_string()),
                        model_id: None,
                        images: None,
                        mentions: None,
                        editor_context: None,
                    })
                    .await
            })
        };
        tokio::task::yield_now().await;
        assert!(!waiting_send.is_finished());

        usecase.interrupt(&session.id).await.unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(INTERRUPT_FORCE_FINALIZE_DELAY).await;
        for _ in 0..100 {
            if usecase.turn_phase(&session.id).await == Some(TurnPhase::Idle)
                && waiting_send.is_finished()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(usecase.turn_phase(&session.id).await, Some(TurnPhase::Idle));
        let queued = waiting_send.await.unwrap().unwrap();
        assert_eq!(queued.pending_queue_count, 1);
        assert!(!first.is_finished());

        usecase.resume_queue(&session.id).await.unwrap();
        for _ in 0..100 {
            if controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurn))
                .count()
                == 2
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            2
        );
        assert!(!first.is_finished());

        first_start_gate.notify_waiters();
        first.await.unwrap().unwrap();
        assert_eq!(
            usecase.turn_phase(&session.id).await,
            Some(TurnPhase::Streaming)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn interrupt_timeout_waits_for_in_flight_permission_event_before_terminal_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let generation = {
            let sessions = usecase.ctx.sessions.lock().await;
            sessions.get(&session_id).unwrap().generation()
        };
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let block_once = Arc::new(std::sync::atomic::AtomicBool::new(true));
        event_notifier.set_streaming_delta_hook({
            let release_rx = Arc::clone(&release_rx);
            let block_once = Arc::clone(&block_once);
            Arc::new(move || {
                if block_once.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    entered_tx.send(()).unwrap();
                    release_rx.lock().unwrap().recv().unwrap();
                }
            })
        });

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-racing-timeout")),
            )
            .unwrap();
        tokio::task::spawn_blocking(move || {
            entered_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("permission event reached its commit path")
        })
        .await
        .unwrap();
        let force_ctx = usecase.ctx.clone();
        let force_session_id = session_id.clone();
        let force = tokio::spawn(async move {
            force_finalize_interrupted_turn(&force_ctx, &force_session_id, generation).await;
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !force.is_finished(),
            "timeout must serialize with the in-flight runtime event"
        );

        release_tx.send(()).unwrap();
        force.await.unwrap();

        let loaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(loaded.turn_phase, TurnPhase::Idle);
        assert!(loaded.pending_permission_request.is_none());
        let changes = event_notifier.state_changes();
        let terminal_index = changes
            .iter()
            .rposition(|change| {
                change.chat_session_id == session_id && change.turn_phase == TurnPhase::Idle
            })
            .expect("timeout terminal notification");
        assert!(!changes.iter().skip(terminal_index + 1).any(|change| {
            change.chat_session_id == session_id
                && (change.turn_phase != TurnPhase::Idle
                    || change.pending_permission_request.is_some())
        }));
    }

    #[tokio::test]
    async fn interrupt_watchdog_is_a_noop_for_a_different_turn_generation() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let stale_generation = {
            let mut sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get_mut(&session_id).unwrap();
            let stale_generation = state.generation();
            let turn_id = state.active_turn_id().expect("active test turn");
            let message_id = state
                .streaming_message_id
                .clone()
                .expect("active test message");
            state.register_turn_start_intent(turn_id, message_id);
            stale_generation
        };

        force_finalize_interrupted_turn(&usecase.ctx, &session_id, stale_generation).await;

        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::Timeout,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn interrupt_fails_before_backend_io_when_durable_append_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        session_store.set_append_event_hook_for_test(Arc::new(|_, event| {
            if matches!(event, AgentSessionEvent::TurnInterruptRequested { .. }) {
                return Err("injected interrupt acceptance append failure".to_string());
            }
            Ok(())
        }));

        let error = usecase.interrupt(&session_id).await.unwrap_err();

        assert!(error
            .to_string()
            .contains("injected interrupt acceptance append failure"));
        let loaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert!(!loaded.queue_paused);
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::Interrupt))
                .count(),
            0
        );
        assert!(!event_notifier
            .state_changes()
            .iter()
            .any(|change| change.queue_paused == Some(true)));
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterruptRequested { .. }
                | AgentSessionEvent::QueuePaused { .. }
        )));

        let restarted =
            crate::test_support::build_agent_runtime_usecase(session_store.clone(), tmp.path());
        assert!(
            !restarted
                .get_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );
    }

    #[tokio::test]
    async fn interrupt_durable_io_does_not_hold_the_runtime_state_mutex() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let first = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let second = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let blocked_session_id = first.session.id.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        session_store.set_append_event_hook_for_test({
            let release_rx = Arc::clone(&release_rx);
            Arc::new(move |session_id, event| {
                if session_id == blocked_session_id
                    && matches!(event, AgentSessionEvent::TurnInterruptRequested { .. })
                {
                    entered_tx.send(()).unwrap();
                    release_rx.lock().unwrap().recv().unwrap();
                }
                Ok(())
            })
        });
        let interrupt_usecase = Arc::clone(&usecase);
        let first_session_id = first.session.id.clone();
        let interrupt =
            tokio::spawn(async move { interrupt_usecase.interrupt(&first_session_id).await });
        tokio::task::spawn_blocking(move || {
            entered_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("durable append hook")
        })
        .await
        .unwrap();

        let other_session = tokio::time::timeout(
            Duration::from_millis(200),
            usecase.get_session(&second.session.id),
        )
        .await
        .expect("another session must remain readable during interrupt commit")
        .unwrap()
        .unwrap();
        assert_eq!(other_session.turn_phase, TurnPhase::Streaming);
        assert_eq!(
            usecase.turn_phase(&first.session.id).await,
            Some(TurnPhase::Streaming)
        );

        release_tx.send(()).unwrap();
        interrupt.await.unwrap().unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn interrupt_acceptance_and_start_failure_preserve_the_crash_terminal_winner() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        let start_gate = controller.pause_next_start_turn();
        controller.fail_next_start_turn();
        let send = {
            let usecase = Arc::clone(&usecase);
            let session_id = session.id.clone();
            let worktree_path = tmp.path().to_string_lossy().to_string();
            tokio::spawn(async move {
                usecase
                    .send_message(SendAgentMessageRequest {
                        chat_session_id: Some(session_id),
                        worktree_path,
                        content: "start fails during stop commit".to_string(),
                        permission_mode: PermissionMode::Edit,
                        plan_mode: false,
                        backend_id: Some("codex".to_string()),
                        model_id: None,
                        images: None,
                        mentions: None,
                        editor_context: None,
                    })
                    .await
            })
        };
        wait_for_start_prompt_count(&controller, &session.id, 1).await;

        let (append_entered_tx, append_entered_rx) = std::sync::mpsc::channel();
        let (release_append_tx, release_append_rx) = std::sync::mpsc::channel();
        let release_append_rx = Arc::new(Mutex::new(release_append_rx));
        let block_once = Arc::new(std::sync::atomic::AtomicBool::new(true));
        session_store.set_append_event_hook_for_test({
            let release_append_rx = Arc::clone(&release_append_rx);
            let block_once = Arc::clone(&block_once);
            Arc::new(move |_, event| {
                if matches!(event, AgentSessionEvent::TurnInterruptRequested { .. })
                    && block_once.swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    append_entered_tx.send(()).unwrap();
                    release_append_rx.lock().unwrap().recv().unwrap();
                }
                Ok(())
            })
        });
        let interrupt = {
            let usecase = Arc::clone(&usecase);
            let session_id = session.id.clone();
            tokio::spawn(async move { usecase.interrupt(&session_id).await })
        };
        tokio::task::spawn_blocking(move || {
            append_entered_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("interrupt append should be blocked")
        })
        .await
        .unwrap();

        start_gate.notify_waiters();
        tokio::task::yield_now().await;
        assert_eq!(
            usecase.turn_phase(&session.id).await,
            Some(TurnPhase::Streaming),
            "start failure must wait behind the durable Stop transition"
        );
        release_append_tx.send(()).unwrap();
        interrupt.await.unwrap().unwrap();
        send.await
            .unwrap()
            .expect("durably accepted Stop owns the concurrent start failure");

        assert_eq!(usecase.turn_phase(&session.id).await, Some(TurnPhase::Idle));
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert!(loaded.queue_paused);
        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::Abort,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::Crash,
                ..
            }
        )));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::TurnInterrupted { .. }))
                .count(),
            1,
            "the shared terminal arbiter must commit exactly one winner",
        );
        assert!(event_notifier.state_changes().iter().any(|change| {
            change.chat_session_id == session.id
                && change.turn_phase == TurnPhase::Idle
                && change.queue_paused == Some(true)
        }));
    }

    #[tokio::test]
    async fn repeated_interrupt_force_finalizes_immediately_and_remains_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;

        usecase.interrupt(&session_id).await.unwrap();
        usecase.interrupt(&session_id).await.unwrap();

        assert_eq!(usecase.turn_phase(&session_id).await, Some(TurnPhase::Idle));
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::TurnInterruptRequested { .. }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::QueuePaused { .. }))
                .count(),
            1
        );
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::TurnInterrupted {
                reason: EventInterruptReason::Timeout,
                ..
            }
        )));
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::Interrupt))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn crash_emits_projected_error_snapshot_before_state_change_and_matches_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![
                    DomainMessagePart::Text {
                        content: "partial output".to_string(),
                        parent_tool_use_id: None,
                    },
                    DomainMessagePart::ToolUse {
                        id: "tool-1".to_string(),
                        tool: "Bash".to_string(),
                        input:
                            crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                                "{}".to_string(),
                            ),
                        parent_tool_use_id: None,
                    },
                ]),
            )
            .unwrap();
        wait_for_stream_delta_count(&event_notifier, 1).await;
        let order_start = event_notifier.event_order().len();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "CLI process exited".to_string(),
                },
            )
            .unwrap();

        wait_for_last_stream_delta(&event_notifier, |delta| {
            delta.snapshot
                && delta.parts.iter().any(|part| {
                    matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
                })
        })
        .await;
        wait_for_error_state_change(&event_notifier, &session_id).await;
        let live = event_notifier
            .streaming_deltas()
            .into_iter()
            .rev()
            .find(|delta| {
                delta.snapshot
                    && delta.parts.iter().any(|part| {
                        matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
                    })
            })
            .unwrap();
        assert!(live.parts.iter().any(|part| {
            matches!(part, MessagePart::Text { content, .. } if content == "partial output")
        }));
        assert!(live.parts.iter().any(|part| {
            matches!(
                part,
                MessagePart::ToolResult {
                    tool_use_id: Some(tool_use_id),
                    is_error: true,
                    ..
                } if tool_use_id == "tool-1"
            )
        }));
        assert_eq!(
            &event_notifier.event_order()[order_start..],
            &["streaming_delta", "state_change"]
        );

        let reloaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(
            reloaded.session.error_reason.as_deref(),
            Some("CLI process exited")
        );
        let persisted = reloaded
            .session
            .messages
            .iter()
            .find(|message| message.id == live.message_id)
            .and_then(|message| message.parts.clone())
            .unwrap();
        assert_eq!(live.parts, persisted);
        let summary = session_store
            .list_sessions(tmp.path(), &reloaded.session.worktree_path)
            .unwrap()
            .into_iter()
            .find(|summary| summary.id == session_id)
            .unwrap();
        assert_eq!(summary.error_reason.as_deref(), Some("CLI process exited"));
    }

    #[tokio::test]
    async fn turn_completed_crash_followed_by_fatal_is_recorded_once() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                    reason: DomainInterruptReason::Crash,
                    error: Some("CLI process exited".to_string()),
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "CLI process exited".to_string(),
                },
            )
            .unwrap();
        wait_for_call_count(&controller, &session_id, TestRuntimeCallKind::Close, 1).await;

        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::TurnInterrupted { .. }))
                .count(),
            1
        );
        assert!(!events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::SessionErrored { .. })));
        let reloaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        let error_contents = reloaded
            .session
            .messages
            .iter()
            .flat_map(|message| message.parts.iter().flatten())
            .filter_map(|part| match part {
                MessagePart::Error { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(error_contents, vec!["CLI process exited"]);
        assert_eq!(
            event_notifier
                .streaming_deltas()
                .iter()
                .filter(|delta| delta.parts.iter().any(|part| {
                    matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
                }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn crash_snapshot_supersedes_older_retry_and_lands_after_notifier_recovers() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        event_notifier.set_streaming_delta_failure(true);
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "partial output".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_emit_failure_state(&usecase, &session_id, |failures, _| failures >= 1)
            .await;

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                    reason: DomainInterruptReason::Crash,
                    error: Some("CLI process exited".to_string()),
                }),
            )
            .unwrap();
        wait_for_error_state_change(&event_notifier, &session_id).await;
        event_notifier.set_streaming_delta_failure(false);
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if event_notifier
                    .delivered_streaming_deltas()
                    .iter()
                    .any(|delta| {
                        delta.snapshot
                            && delta.parts.iter().any(|part| {
                                matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
                            })
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        let delivered = event_notifier.delivered_streaming_deltas();
        let terminal = delivered
            .iter()
            .find(|delta| {
                delta.parts.iter().any(|part| {
                    matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
                })
            })
            .unwrap();
        assert!(terminal.parts.iter().any(|part| {
            matches!(part, MessagePart::Text { content, .. } if content == "partial output")
        }));
    }

    #[tokio::test]
    async fn successful_crash_snapshot_cancels_pre_final_retry_before_delayed_flush() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let partial_parts = vec![MessagePart::Text {
            content: "partial output".to_string(),
            parent_tool_use_id: None,
        }];
        {
            let mut sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get_mut(&session_id).unwrap();
            state.restore_stream_buffer_for_test(Vec::new(), partial_parts.clone(), true);
        }
        // The pre-final flush fails, then the authoritative crash snapshot succeeds.
        event_notifier.set_streaming_delta_outcomes([false, true]);

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                    reason: DomainInterruptReason::Crash,
                    error: Some("CLI process exited".to_string()),
                }),
            )
            .unwrap();
        wait_for_error_state_change(&event_notifier, &session_id).await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let attempted = event_notifier.streaming_deltas();
        assert_eq!(attempted.len(), 2);
        let delivered = event_notifier.delivered_streaming_deltas();
        assert_eq!(delivered.len(), 1);
        let terminal = delivered.last().unwrap();
        assert_eq!(terminal.parts.first(), partial_parts.first());
        assert!(terminal.parts.iter().any(|part| {
            matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
        }));
    }

    #[tokio::test]
    async fn crash_snapshot_retry_survives_queued_turn_reset_and_lands_after_recovery() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let session_id = enqueue_second_turn_for_test(
            &usecase,
            &controller,
            tmp.path().to_string_lossy().to_string(),
        )
        .await;
        event_notifier.set_streaming_delta_failure(true);

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "CLI process exited".to_string(),
                },
            )
            .unwrap();
        wait_for_error_state_change(&event_notifier, &session_id).await;
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .into_iter()
                .filter(|kind| *kind == TestRuntimeCallKind::StartTurn)
                .count(),
            1,
            "Fatal must keep the queued turn paused until explicit resume"
        );
        assert_eq!(usecase.turn_phase(&session_id).await, Some(TurnPhase::Idle));

        usecase.resume_queue(&session_id).await.unwrap();
        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );

        event_notifier.set_streaming_delta_failure(false);
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if event_notifier
                    .delivered_streaming_deltas()
                    .iter()
                    .any(|delta| {
                        delta.parts.iter().any(|part| {
                            matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
                        })
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn idle_fatal_is_durable_live_and_survives_later_projection() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if event_notifier.state_changes().iter().any(|change| {
                    change.chat_session_id == session_id
                        && change.session_state == Some(SessionState::Done)
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let order_start = event_notifier.event_order().len();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "app server stopped".to_string(),
                },
            )
            .unwrap();

        wait_for_last_stream_delta(&event_notifier, |delta| {
            delta.snapshot
                && delta.message.as_ref().is_some_and(|message| {
                    message.parts.as_deref()
                        == Some(
                            [MessagePart::Error {
                                content: "app server stopped".to_string(),
                                parent_tool_use_id: None,
                            }]
                            .as_slice(),
                        )
                })
        })
        .await;
        wait_for_error_state_change(&event_notifier, &session_id).await;
        assert!(event_notifier
            .state_changes()
            .iter()
            .rev()
            .find(|change| change.session_state == Some(SessionState::Error))
            .is_some_and(|change| change.completed_at.is_none()));
        assert_eq!(
            &event_notifier.event_order()[order_start..],
            &["streaming_delta", "state_change"]
        );
        let live = event_notifier
            .streaming_deltas()
            .into_iter()
            .find(|delta| delta.message.is_some())
            .unwrap();
        assert_eq!(
            live.parts,
            vec![MessagePart::Error {
                content: "app server stopped".to_string(),
                parent_tool_use_id: None,
            }]
        );
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::SessionErrored { reason, .. } if reason == "app server stopped"
        )));

        let reloaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(reloaded.session.state, SessionState::Error);
        assert_eq!(
            reloaded.session.error_reason.as_deref(),
            Some("app server stopped")
        );
        let persisted = reloaded
            .session
            .messages
            .iter()
            .find(|message| message.id == live.message_id)
            .and_then(|message| message.parts.clone())
            .unwrap();
        assert_eq!(live.parts, persisted);
        let live_timestamp = live.message.as_ref().unwrap().timestamp;
        let reloaded_timestamp = reloaded
            .session
            .messages
            .iter()
            .find(|message| message.id == live.message_id)
            .unwrap()
            .timestamp;
        assert!((live_timestamp - reloaded_timestamp).abs() < 1e-6);
        let summary = session_store
            .list_sessions(tmp.path(), &reloaded.session.worktree_path)
            .unwrap()
            .into_iter()
            .find(|summary| summary.id == session_id)
            .unwrap();
        assert_eq!(summary.error_reason.as_deref(), Some("app server stopped"));

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "app server stopped again".to_string(),
                },
            )
            .unwrap();
        wait_for_last_stream_delta(&event_notifier, |delta| {
            delta.message.as_ref().is_some_and(|message| {
                message.parts.as_deref()
                    == Some(
                        [MessagePart::Error {
                            content: "app server stopped again".to_string(),
                            parent_tool_use_id: None,
                        }]
                        .as_slice(),
                    )
            })
        })
        .await;
        let second_live = event_notifier.streaming_deltas().last().cloned().unwrap();
        assert_ne!(live.message_id, second_live.message_id);

        let after_second_fatal = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(
            after_second_fatal.session.error_reason.as_deref(),
            Some("app server stopped again")
        );
        let persisted_error_ids = after_second_fatal
            .session
            .messages
            .iter()
            .filter(|message| {
                message.parts.as_ref().is_some_and(|parts| {
                    parts
                        .iter()
                        .any(|part| matches!(part, MessagePart::Error { .. }))
                })
            })
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            persisted_error_ids,
            vec![live.message_id.as_str(), second_live.message_id.as_str()]
        );

        session_store
            .append_session_event_and_project_state(
                tmp.path(),
                &session_id,
                AgentSessionEvent::ToolCallRetried {
                    turn_id: 99,
                    tool_use_id: "unrelated".to_string(),
                    attempt: 1,
                },
            )
            .unwrap();
        let after_reprojection = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(after_reprojection.session.state, SessionState::Error);
        assert_eq!(
            after_reprojection.session.error_reason.as_deref(),
            Some("app server stopped again")
        );
    }

    #[tokio::test]
    async fn distinct_idle_fatal_retries_land_in_message_order_after_notifier_recovery() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if event_notifier.state_changes().iter().any(|change| {
                    change.chat_session_id == session_id
                        && change.session_state == Some(SessionState::Done)
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let delivered_before = event_notifier.delivered_streaming_deltas().len();
        event_notifier.set_streaming_delta_failure(true);

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "first fatal".to_string(),
                },
            )
            .unwrap();
        wait_for_last_stream_delta(&event_notifier, |delta| {
            delta.message.as_ref().is_some_and(|message| {
                message.parts.as_deref()
                    == Some(
                        [MessagePart::Error {
                            content: "first fatal".to_string(),
                            parent_tool_use_id: None,
                        }]
                        .as_slice(),
                    )
            })
        })
        .await;

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "second fatal".to_string(),
                },
            )
            .unwrap();
        let persisted_error_ids = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let reloaded = usecase.get_session(&session_id).await.unwrap().unwrap();
                if reloaded.session.error_reason.as_deref() == Some("second fatal") {
                    break reloaded
                        .session
                        .messages
                        .iter()
                        .filter(|message| {
                            message.parts.as_ref().is_some_and(|parts| {
                                parts
                                    .iter()
                                    .any(|part| matches!(part, MessagePart::Error { .. }))
                            })
                        })
                        .map(|message| message.id.clone())
                        .collect::<Vec<_>>();
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(persisted_error_ids.len(), 2);
        assert_eq!(
            event_notifier.delivered_streaming_deltas().len(),
            delivered_before
        );

        event_notifier.set_streaming_delta_failure(false);
        let delivered_ids = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let delivered = event_notifier.delivered_streaming_deltas();
                if delivered.len() >= delivered_before + 2 {
                    break delivered[delivered_before..]
                        .iter()
                        .map(|delta| delta.message_id.clone())
                        .collect::<Vec<_>>();
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(delivered_ids, persisted_error_ids);
    }

    #[derive(Clone, Copy)]
    enum IdleFatalPersistenceFailure {
        AppendEvent,
        AppendMessage,
        ProjectMeta,
    }

    async fn assert_idle_fatal_persistence_failure_retries_exact_episode(
        failure: IdleFatalPersistenceFailure,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if event_notifier.state_changes().iter().any(|change| {
                    change.chat_session_id == session_id
                        && change.session_state == Some(SessionState::Done)
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        let failed = Arc::new(AtomicBool::new(false));
        match failure {
            IdleFatalPersistenceFailure::AppendEvent => {
                let failed = Arc::clone(&failed);
                session_store.set_append_event_hook_for_test(Arc::new(move |_, event| {
                    if matches!(event, AgentSessionEvent::SessionErrored { .. })
                        && !failed.swap(true, Ordering::SeqCst)
                    {
                        Err("injected session error event failure".to_string())
                    } else {
                        Ok(())
                    }
                }));
            }
            IdleFatalPersistenceFailure::AppendMessage => {
                let failed = Arc::clone(&failed);
                session_store.set_append_message_hook_for_test(Arc::new(move |_, message| {
                    if message.parts.as_ref().is_some_and(|parts| {
                        parts
                            .iter()
                            .any(|part| matches!(part, MessagePart::Error { .. }))
                    }) && !failed.swap(true, Ordering::SeqCst)
                    {
                        Err("injected session error message failure".to_string())
                    } else {
                        Ok(())
                    }
                }));
            }
            IdleFatalPersistenceFailure::ProjectMeta => {
                let failed = Arc::clone(&failed);
                session_store.set_projection_hook_for_test(Arc::new(move |_, state, _| {
                    if state == &SessionState::Error && !failed.swap(true, Ordering::SeqCst) {
                        Err("injected session error projection failure".to_string())
                    } else {
                        Ok(())
                    }
                }));
            }
        }
        let delta_start = event_notifier.streaming_deltas().len();
        let state_start = event_notifier.state_changes().len();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "app server stopped".to_string(),
                },
            )
            .unwrap();
        wait_for_call(&controller, &session_id, TestRuntimeCallKind::Close).await;

        let reloaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert!(failed.load(Ordering::SeqCst));
        assert_eq!(reloaded.session.state, SessionState::Error);
        assert!(reloaded.session.messages.iter().any(|message| {
            message.parts.as_ref().is_some_and(|parts| {
                parts.iter().any(|part| {
                    matches!(part, MessagePart::Error { content, .. } if content == "app server stopped")
                })
            })
        }));
        assert_eq!(
            session_store
                .load_session_events(tmp.path(), &session_id)
                .unwrap()
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::SessionErrored { .. }))
                .count(),
            1
        );
        assert!(event_notifier.streaming_deltas()[delta_start..]
            .iter()
            .any(|delta| delta.parts.iter().any(|part| {
                matches!(part, MessagePart::Error { content, .. } if content == "app server stopped")
            })));
        assert!(event_notifier.state_changes()[state_start..]
            .iter()
            .any(|change| change.session_state == Some(SessionState::Error)));
    }

    #[tokio::test]
    async fn idle_fatal_append_event_failure_retries_one_error_episode() {
        assert_idle_fatal_persistence_failure_retries_exact_episode(
            IdleFatalPersistenceFailure::AppendEvent,
        )
        .await;
    }

    #[tokio::test]
    async fn idle_fatal_append_message_failure_retries_one_error_episode() {
        assert_idle_fatal_persistence_failure_retries_exact_episode(
            IdleFatalPersistenceFailure::AppendMessage,
        )
        .await;
    }

    #[tokio::test]
    async fn idle_fatal_meta_projection_failure_retries_one_error_episode() {
        assert_idle_fatal_persistence_failure_retries_exact_episode(
            IdleFatalPersistenceFailure::ProjectMeta,
        )
        .await;
    }

    #[derive(Clone, Copy)]
    enum CrashPersistenceFailure {
        AppendEvent,
        PersistParts,
    }

    async fn assert_crash_persistence_failure_retries_exact_terminal(
        failure: CrashPersistenceFailure,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        match failure {
            CrashPersistenceFailure::AppendEvent => {
                let attempts = Arc::clone(&attempts);
                session_store.set_append_event_hook_for_test(Arc::new(move |_, event| {
                    if matches!(event, AgentSessionEvent::FinalPartsRecorded { .. }) {
                        let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                        if attempt < PERSIST_MAX_ATTEMPTS {
                            return Err("injected final event failure".to_string());
                        }
                    }
                    Ok(())
                }));
            }
            CrashPersistenceFailure::PersistParts => {
                let attempts = Arc::clone(&attempts);
                session_store.set_persist_parts_hook_for_test(Arc::new(move |_, _, _| {
                    let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                    if attempt < PERSIST_MAX_ATTEMPTS {
                        return Err("injected final parts failure".to_string());
                    }
                    Ok(())
                }));
            }
        }
        let delta_start = event_notifier.streaming_deltas().len();
        let state_start = event_notifier.state_changes().len();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "CLI process exited".to_string(),
                },
            )
            .unwrap();
        wait_for_call(&controller, &session_id, TestRuntimeCallKind::Close).await;

        let reloaded = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(reloaded.session.state, SessionState::Error);
        assert!(
            attempts.load(Ordering::SeqCst) > PERSIST_MAX_ATTEMPTS,
            "the retained event must be retried after the bounded terminal helper exhausts its attempts"
        );
        assert!(reloaded.session.messages.iter().any(|message| {
            message.parts.as_ref().is_some_and(|parts| {
                parts.iter().any(|part| {
                    matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
                })
            })
        }));
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::TurnInterrupted { .. })));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentSessionEvent::TurnInterrupted { .. }))
                .count(),
            1
        );
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::FinalPartsRecorded { .. })));
        assert!(event_notifier.streaming_deltas()[delta_start..]
            .iter()
            .any(|delta| delta.parts.iter().any(|part| {
                matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
            })));
        assert!(event_notifier.state_changes()[state_start..]
            .iter()
            .any(|change| change.session_state == Some(SessionState::Error)));
    }

    #[tokio::test]
    async fn crash_append_event_failure_retries_one_terminal_without_loss() {
        assert_crash_persistence_failure_retries_exact_terminal(
            CrashPersistenceFailure::AppendEvent,
        )
        .await;
    }

    #[tokio::test]
    async fn crash_persist_parts_failure_retries_one_terminal_without_loss() {
        assert_crash_persistence_failure_retries_exact_terminal(
            CrashPersistenceFailure::PersistParts,
        )
        .await;
    }

    #[tokio::test]
    async fn fatal_closes_runtime_and_pauses_queued_turn_until_explicit_resume() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let worktree_path = tmp.path().to_string_lossy().to_string();
        let first = usecase
            .send_message(send_request(worktree_path.clone()))
            .await
            .unwrap();
        let session_id = first.session.id.clone();
        wait_for_start_prompt_count(&controller, &session_id, 1).await;
        let second = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path,
                content: "queued after fatal".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        assert_eq!(second.pending_queue_count, 1);
        controller.pause_start_turn();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "fatal test".to_string(),
                },
            )
            .unwrap();

        wait_for_call(&controller, &session_id, TestRuntimeCallKind::Close).await;
        assert!(event_notifier.state_changes().iter().any(|change| {
            change.chat_session_id == session_id
                && change.turn_phase == TurnPhase::Idle
                && change.queue_paused == Some(true)
                && change.session_state == Some(SessionState::Error)
        }));
        assert_eq!(usecase.pending_queue(&session_id).await.len(), 1);
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::StartTurn))
                .count(),
            1,
            "fatal must not automatically submit the queued turn"
        );

        controller.release_start_turn();
        usecase.resume_queue(&session_id).await.unwrap();
        wait_for_start_prompt_count(&controller, &session_id, 2).await;
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if usecase.pending_queue(&session_id).await.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );
    }

    #[tokio::test]
    async fn test_workflow_turn_complete通知は_session_lock外でdispatchされる() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store,
                tmp.path(),
            );
        let worktree_path = tmp.path().to_string_lossy().to_string();
        let response = usecase
            .send_message(send_request(worktree_path.clone()))
            .await
            .unwrap();
        wait_for_start_prompt_count(&controller, &response.session.id, 1).await;

        let done = Arc::new(Notify::new());
        usecase.set_workflow_turn_complete_notifier(Arc::new(ReentrantWorkflowNotifier {
            usecase: Arc::clone(&usecase),
            session_id: response.session.id.clone(),
            worktree_path,
            done: Arc::clone(&done),
        }));
        controller
            .emit(
                &response.session.id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), done.notified())
            .await
            .expect("workflow notification must be able to re-enter same session");
        wait_for_start_prompt_count(&controller, &response.session.id, 2).await;
    }

    #[tokio::test]
    async fn test_init_sessionsは_workflow_node_tabを復元し_active_session_modeを返す() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller, workspace_query) =
            build_agent_runtime_usecase_with_controller_and_workspace_query(
                session_store.clone(),
                tmp.path(),
            );
        let regular = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Ask,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: true,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        let step = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, None)),
            },
        )
        .unwrap();
        workspace_query
            .replace_session_summaries(vec![regular.to_summary(), step.to_summary()], Vec::new());
        let open_tabs = OpenTabRegistry::default();

        let response = usecase
            .init_sessions(tmp.path().to_string_lossy().as_ref(), &open_tabs)
            .await
            .unwrap();

        assert!(open_tabs.contains(&step.id));
        assert!(!open_tabs.contains(&regular.id));
        assert_eq!(response.permission_mode, PermissionMode::Ask.as_str());
        assert!(response.plan_mode);
        assert_eq!(
            response
                .active_session
                .as_ref()
                .map(|session| session.session.id.as_str()),
            Some(regular.id.as_str())
        );
    }

    #[derive(Default)]
    struct RecordingAcceptedSendObligationDriver {
        reconciliations: Mutex<Vec<(String, String)>>,
        running: Mutex<Vec<(String, String, u64)>>,
        recovery_wake: Mutex<Option<AcceptedSendRecoveryWake>>,
    }

    #[async_trait::async_trait]
    impl AcceptedSendObligationDriver for RecordingAcceptedSendObligationDriver {
        async fn claim_immediate_turn_execution(
            &self,
            _operation_id: &str,
            _obligation_id: &str,
        ) -> Result<AcceptedSendExecutionClaim, ()> {
            Ok(AcceptedSendExecutionClaim::new(|| {}))
        }

        async fn claim_queued_turn_execution(
            &self,
            _operation_id: &str,
            _obligation_id: &str,
            _session_id: &str,
            _queue_item_id: &str,
            _event: AgentSessionEvent,
        ) -> Result<super::super::ports::AcceptedQueuedTurnExecutionClaimOutcome, ()> {
            Ok(
                super::super::ports::AcceptedQueuedTurnExecutionClaimOutcome::Claimed(
                    super::super::ports::AcceptedSendExecutionClaim::new(|| {}),
                ),
            )
        }

        async fn mark_turn_running(
            &self,
            operation_id: &str,
            obligation_id: &str,
            turn_id: u64,
        ) -> Result<(), ()> {
            self.running.lock().unwrap().push((
                operation_id.to_string(),
                obligation_id.to_string(),
                turn_id,
            ));
            Ok(())
        }

        async fn reconcile_turn_execution(
            &self,
            operation_id: &str,
            obligation_id: &str,
        ) -> Option<super::super::ports::AcceptedSendRecoveryWake> {
            self.reconciliations
                .lock()
                .unwrap()
                .push((operation_id.to_string(), obligation_id.to_string()));
            self.recovery_wake.lock().unwrap().take()
        }
    }

    #[tokio::test]
    async fn queued_driver_reconciliation_wakes_after_the_complete_claim_release_chain() {
        let first_release_observed = Arc::new(AtomicBool::new(false));
        let final_release_observed = Arc::new(AtomicBool::new(false));
        let wake_observed = Arc::new(AtomicBool::new(false));
        let claim = AcceptedSendExecutionClaim::new({
            let first_release_observed = Arc::clone(&first_release_observed);
            move || first_release_observed.store(true, Ordering::SeqCst)
        })
        .release_then({
            let first_release_observed = Arc::clone(&first_release_observed);
            let final_release_observed = Arc::clone(&final_release_observed);
            move || {
                assert!(
                    first_release_observed.load(Ordering::SeqCst),
                    "the driver's original claim release must run first"
                );
                final_release_observed.store(true, Ordering::SeqCst);
            }
        });
        let driver = RecordingAcceptedSendObligationDriver {
            recovery_wake: Mutex::new(Some(AcceptedSendRecoveryWake::new({
                let first_release_observed = Arc::clone(&first_release_observed);
                let final_release_observed = Arc::clone(&final_release_observed);
                let wake_observed = Arc::clone(&wake_observed);
                move || {
                    assert!(first_release_observed.load(Ordering::SeqCst));
                    assert!(
                        final_release_observed.load(Ordering::SeqCst),
                        "queued dispatch release must precede the recovery wake"
                    );
                    wake_observed.store(true, Ordering::SeqCst);
                }
            }))),
            ..Default::default()
        };
        let mut accepted_claim = Some(claim);

        arm_accepted_send_recovery_after_claim_release(
            &driver,
            "queued-reconcile",
            "queued-reconcile.exec",
            &mut accepted_claim,
        )
        .await;

        assert!(!first_release_observed.load(Ordering::SeqCst));
        assert!(!final_release_observed.load(Ordering::SeqCst));
        assert!(!wake_observed.load(Ordering::SeqCst));
        drop(accepted_claim.take());
        assert!(first_release_observed.load(Ordering::SeqCst));
        assert!(final_release_observed.load(Ordering::SeqCst));
        assert!(wake_observed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn accepted_immediate_send_keeps_execution_identity_on_current_turn() {
        let tmp = tempfile::tempdir().unwrap();
        let local_store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
        session_store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);
        let human_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Human,
            "accepted prompt",
            None,
            None,
        )
        .unwrap();
        let agent_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            None,
            None,
        )
        .unwrap();

        usecase
            .execute_accepted_send(AcceptedSendExecution {
                request: AcceptedRuntimeSendInput {
                    content: "accepted prompt".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    images: Vec::new(),
                    mentions: Vec::new(),
                    editor_context: None,
                    base_system_prompt: None,
                    workflow_instructions: Vec::new(),
                },
                operation_id: "send-operation-1",
                execution_obligation_id: "send-operation-1.execute",
                session_id: &session.id,
                human_message_id: &human_message.id,
                assistant_message_id: Some(&agent_message.id),
                disposition: crate::domain::agent_session::events::SendDisposition::StartedTurn {
                    turn_id: "1".to_string(),
                },
                reserved_turn_id: None,
            })
            .await
            .unwrap();

        let (operation_id, execution_obligation_id) = {
            let sessions = usecase.ctx.sessions.lock().await;
            let current_turn = sessions
                .get(&session.id)
                .and_then(|state| state.current_turn_input.as_ref())
                .expect("accepted turn input remains recoverable");
            (
                current_turn.accepted_operation_id.clone(),
                current_turn.execution_obligation_id.clone(),
            )
        };
        assert_eq!(operation_id.as_deref(), Some("send-operation-1"));
        assert_eq!(
            execution_obligation_id.as_deref(),
            Some("send-operation-1.execute")
        );
        assert!(controller.call_kinds_for(&session.id).contains(
            &TestRuntimeCallKind::StartTurnPrompt {
                prompt: "accepted prompt".to_string(),
            }
        ));
    }

    #[tokio::test]
    async fn accepted_turn_backend_recovery_submits_input_before_provider_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let local_store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
        session_store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let obligation_driver = Arc::new(RecordingAcceptedSendObligationDriver::default());
        usecase.set_accepted_send_obligation_driver(obligation_driver.clone());
        let session =
            provider_establish_test_session(&session_store, tmp.path(), Some("dead-provider"));
        add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Human,
            "previous context",
            None,
            None,
        )
        .unwrap();
        add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Agent,
            "previous answer",
            None,
            None,
        )
        .unwrap();
        let human_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Human,
            "recover accepted prompt",
            None,
            None,
        )
        .unwrap();
        let agent_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            None,
            None,
        )
        .unwrap();
        controller.fail_next_resume_open();

        usecase
            .execute_accepted_send(AcceptedSendExecution {
                request: AcceptedRuntimeSendInput {
                    content: "recover accepted prompt".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    images: Vec::new(),
                    mentions: Vec::new(),
                    editor_context: None,
                    base_system_prompt: None,
                    workflow_instructions: Vec::new(),
                },
                operation_id: "send-recovery-operation",
                execution_obligation_id: "send-recovery-operation.exec",
                session_id: &session.id,
                human_message_id: &human_message.id,
                assistant_message_id: Some(&agent_message.id),
                disposition: crate::domain::agent_session::events::SendDisposition::StartedTurn {
                    turn_id: "1".to_string(),
                },
                reserved_turn_id: None,
            })
            .await
            .unwrap();

        let calls_before_identity = controller.call_kinds_for(&session.id);
        assert_eq!(
            calls_before_identity
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            2,
            "the dead resume is replaced exactly once"
        );
        let replacement_prompts = calls_before_identity
            .iter()
            .filter_map(|call| match call {
                TestRuntimeCallKind::StartTurnPrompt { prompt } => Some(prompt),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(replacement_prompts.len(), 1);
        assert!(replacement_prompts[0].contains("previous context"));
        assert!(replacement_prompts[0].contains("previous answer"));
        assert!(replacement_prompts[0].ends_with("recover accepted prompt"));
        assert!(!usecase.provider_session_is_confirmed(&session.id).await);
        assert!(
            usecase
                .owns_accepted_turn_execution(
                    &session.id,
                    "send-recovery-operation",
                    "send-recovery-operation.exec",
                )
                .await
        );
        assert!(usecase.pending_queue(&session.id).await.is_empty());
        tokio::time::timeout(Duration::from_secs(1), async {
            while obligation_driver.running.lock().unwrap().is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            obligation_driver.running.lock().unwrap().as_slice(),
            &[(
                "send-recovery-operation".to_string(),
                "send-recovery-operation.exec".to_string(),
                1,
            )]
        );

        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "replacement-provider".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !usecase.provider_session_is_confirmed(&session.id).await {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::StartTurn))
                .count(),
            1,
            "provider identity completion must not enqueue or submit the turn again"
        );
        let recovered_meta = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            recovered_meta.agent_session_id.as_deref(),
            Some("replacement-provider")
        );
        assert_eq!(
            recovered_meta.context_carry,
            Some(ContextCarryState::Reinjected)
        );
        assert_eq!(recovered_meta.context_reinjection_generation, None);
    }

    #[tokio::test]
    async fn accepted_turn_backend_lost_event_restarts_without_lock_reentry_or_second_claim() {
        let tmp = tempfile::tempdir().unwrap();
        let local_store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
        session_store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let obligation_driver = Arc::new(RecordingAcceptedSendObligationDriver::default());
        usecase.set_accepted_send_obligation_driver(obligation_driver.clone());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);
        let human_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Human,
            "continue exact accepted turn",
            None,
            None,
        )
        .unwrap();
        let agent_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            None,
            None,
        )
        .unwrap();

        usecase
            .execute_accepted_send(AcceptedSendExecution {
                request: AcceptedRuntimeSendInput {
                    content: "continue exact accepted turn".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    images: Vec::new(),
                    mentions: Vec::new(),
                    editor_context: None,
                    base_system_prompt: None,
                    workflow_instructions: Vec::new(),
                },
                operation_id: "send-backend-lost-operation",
                execution_obligation_id: "send-backend-lost-operation.exec",
                session_id: &session.id,
                human_message_id: &human_message.id,
                assistant_message_id: Some(&agent_message.id),
                disposition: crate::domain::agent_session::events::SendDisposition::StartedTurn {
                    turn_id: "1".to_string(),
                },
                reserved_turn_id: None,
            })
            .await
            .unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 1).await;

        controller
            .emit(&session.id, AgentRuntimeEvent::BackendSessionCleared)
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;
        wait_for_start_prompt_count(&controller, &session.id, 2).await;

        assert!(
            usecase
                .owns_accepted_turn_execution(
                    &session.id,
                    "send-backend-lost-operation",
                    "send-backend-lost-operation.exec",
                )
                .await
        );
        assert!(usecase.pending_queue(&session.id).await.is_empty());
        assert_eq!(
            obligation_driver.running.lock().unwrap().as_slice(),
            &[(
                "send-backend-lost-operation".to_string(),
                "send-backend-lost-operation.exec".to_string(),
                1,
            )]
        );
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|call| matches!(
                    call,
                    TestRuntimeCallKind::StartTurnPrompt { prompt }
                        if prompt == "continue exact accepted turn"
                ))
                .count(),
            2,
            "one original submission and one replacement-runtime continuation are expected"
        );

        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "replacement-after-loss".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !usecase.provider_session_is_confirmed(&session.id).await {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::StartTurn))
                .count(),
            2,
            "identity completion must not submit a third input"
        );
        assert_eq!(
            session_store
                .get_session_meta(tmp.path(), &session.id)
                .unwrap()
                .unwrap()
                .context_reinjection_generation,
            None
        );
    }

    #[tokio::test]
    async fn accepted_stop_fences_late_backend_loss_without_reopening_the_turn() {
        let tmp = tempfile::tempdir().unwrap();
        let local_store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
        session_store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);
        let human_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Human,
            "stop this accepted turn",
            None,
            None,
        )
        .unwrap();
        let agent_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            None,
            None,
        )
        .unwrap();

        usecase
            .execute_accepted_send(AcceptedSendExecution {
                request: AcceptedRuntimeSendInput {
                    content: "stop this accepted turn".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    images: Vec::new(),
                    mentions: Vec::new(),
                    editor_context: None,
                    base_system_prompt: None,
                    workflow_instructions: Vec::new(),
                },
                operation_id: "send-stop-race-operation",
                execution_obligation_id: "send-stop-race-operation.exec",
                session_id: &session.id,
                human_message_id: &human_message.id,
                assistant_message_id: Some(&agent_message.id),
                disposition: crate::domain::agent_session::events::SendDisposition::StartedTurn {
                    turn_id: "1".to_string(),
                },
                reserved_turn_id: None,
            })
            .await
            .unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 1).await;

        let runtime_epoch = {
            let sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get(&session.id).expect("active runtime state");
            assert!(!state.queue_is_paused());
            assert!(!state.interrupt_requested_for_current());
            state.runtime_epoch()
        };
        let accepted_at = crate::usecase::agent_session::session::now_timestamp();
        append_session_events_blocking(
            &usecase.ctx,
            &session.id,
            vec![
                AgentSessionEvent::TurnInterruptRequested {
                    turn_id: 1,
                    at: accepted_at,
                },
                AgentSessionEvent::QueuePaused { at: accepted_at },
            ],
        )
        .await
        .unwrap();

        // The durable acceptance closes the interval before the production
        // gate can install its process-local fence.
        apply_runtime_event(
            &usecase.ctx,
            &session.id,
            runtime_epoch,
            crate::usecase::agent_session::session::now_timestamp(),
            AgentRuntimeEvent::BackendSessionCleared,
        )
        .await
        .unwrap();

        usecase
            .interrupt_provider_effect_after_stop_acceptance(&session.id, 1)
            .await
            .unwrap();
        {
            let sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get(&session.id).expect("active runtime state");
            assert!(state.queue_is_paused());
			assert_eq!(state.queue_paused_at(), Some(accepted_at));
            assert!(
                state.interrupt_requested_for_current(),
                "durable Stop must fence the exact active process generation"
            );
        }
        apply_runtime_event(
            &usecase.ctx,
            &session.id,
            runtime_epoch,
            crate::usecase::agent_session::session::now_timestamp(),
            AgentRuntimeEvent::BackendSessionCleared,
        )
        .await
        .unwrap();

        let calls = controller.call_kinds_for(&session.id);
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            1,
            "a provider-loss event after Stop acceptance must not open a replacement runtime"
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::StartTurn))
                .count(),
            1,
            "a provider-loss event after Stop acceptance must not resubmit the input"
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::Interrupt))
                .count(),
            1
        );
        assert!(!session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryStarted { .. }
            )));
    }

    #[tokio::test]
    async fn accepted_turn_event_recovery_open_failure_reconciles_the_exact_execution() {
        let tmp = tempfile::tempdir().unwrap();
        let local_store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
        session_store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let driver = Arc::new(RecordingAcceptedSendObligationDriver::default());
        usecase.set_accepted_send_obligation_driver(driver.clone());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);
        let human_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Human,
            "accepted turn whose replacement fails",
            None,
            None,
        )
        .unwrap();
        let agent_message = add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            None,
            None,
        )
        .unwrap();

        usecase
            .execute_accepted_send(AcceptedSendExecution {
                request: AcceptedRuntimeSendInput {
                    content: "accepted turn whose replacement fails".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    images: Vec::new(),
                    mentions: Vec::new(),
                    editor_context: None,
                    base_system_prompt: None,
                    workflow_instructions: Vec::new(),
                },
                operation_id: "send-replacement-open-failure",
                execution_obligation_id: "send-replacement-open-failure.exec",
                session_id: &session.id,
                human_message_id: &human_message.id,
                assistant_message_id: Some(&agent_message.id),
                disposition: crate::domain::agent_session::events::SendDisposition::StartedTurn {
                    turn_id: "1".to_string(),
                },
                reserved_turn_id: None,
            })
            .await
            .unwrap();
        controller.fail_next_open();
        controller
            .emit(&session.id, AgentRuntimeEvent::BackendSessionCleared)
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let reconciled = !driver.reconciliations.lock().unwrap().is_empty();
                if reconciled && usecase.turn_phase(&session.id).await == Some(TurnPhase::Idle) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            *driver.reconciliations.lock().unwrap(),
            vec![(
                "send-replacement-open-failure".to_string(),
                "send-replacement-open-failure.exec".to_string(),
            )]
        );
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::StartTurn))
                .count(),
            1,
            "a failed replacement open must not submit the input again"
        );
    }

    #[tokio::test]
    async fn legacy_send_leaves_current_turn_without_accepted_execution_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            build_agent_runtime_usecase_with_controller(session_store, tmp.path());

        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();

        let sessions = usecase.ctx.sessions.lock().await;
        let current_turn = sessions
            .get(&response.session.id)
            .and_then(|state| state.current_turn_input.as_ref())
            .expect("legacy turn input remains available");
        assert!(current_turn.accepted_operation_id.is_none());
        assert!(current_turn.execution_obligation_id.is_none());
    }

    #[tokio::test]
    async fn provider_establishment_observation_retries_metadata_commit_without_reopening() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = provider_establish_test_session(&session_store, tmp.path(), None);
        let commit_attempts = Arc::new(AtomicUsize::new(0));
        session_store.set_backend_established_hook_for_test(Arc::new({
            let commit_attempts = Arc::clone(&commit_attempts);
            move |_, _| {
                if commit_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Err("injected provider metadata failure".to_string());
                }
                Ok(())
            }
        }));

        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 1).await;
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "provider-session-1".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            while !usecase.provider_session_is_confirmed(&session.id).await {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        let meta = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(meta.agent_session_id.as_deref(), Some("provider-session-1"));
        assert_eq!(meta.provider_session_generation, 1);
        assert_eq!(commit_attempts.load(Ordering::SeqCst), 2);
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn provider_establishment_commit_reply_loss_replays_exact_observation_once() {
        let tmp = tempfile::tempdir().unwrap();
        let local_store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
        session_store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);

        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 1).await;
        local_store
            .fault_injector()
            .arm_crash_after_commit_before_readback();
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "provider-after-reply-loss".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            while !usecase.provider_session_is_confirmed(&session.id).await {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        let meta = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            meta.agent_session_id.as_deref(),
            Some("provider-after-reply-loss")
        );
        assert_eq!(meta.provider_session_generation, 1);
        assert!(
            meta.provider_session_observation_id.is_some(),
            "the durable generation must retain the exact replay identity"
        );
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            1,
            "metadata reply loss must not reopen the provider"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn provider_establishment_persistence_does_not_hold_runtime_event_locks() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);
        let persistence_started = Arc::new(AtomicBool::new(false));
        let release_persistence = Arc::new(AtomicBool::new(false));
        session_store.set_backend_established_hook_for_test(Arc::new({
            let persistence_started = Arc::clone(&persistence_started);
            let release_persistence = Arc::clone(&release_persistence);
            move |_, _| {
                persistence_started.store(true, Ordering::SeqCst);
                while !release_persistence.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ok(())
            }
        }));

        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 1).await;
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "provider-blocked-persistence".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            while !persistence_started.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let lock_result = tokio::time::timeout(Duration::from_millis(250), async {
            let _session_guard = usecase.ctx.session_locks.acquire(&session.id).await;
            let _runtime_event_guard = usecase.ctx.runtime_event_locks.acquire(&session.id).await;
        })
        .await;
        release_persistence.store(true, Ordering::SeqCst);
        assert!(
            lock_result.is_ok(),
            "provider metadata I/O must not retain either runtime-event serialization lock"
        );

        tokio::time::timeout(Duration::from_secs(1), async {
            while !usecase.provider_session_is_confirmed(&session.id).await {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn provider_establishment_lifecycle_fence_clears_exact_pending_observation() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);
        let commit_attempts = Arc::new(AtomicUsize::new(0));
        session_store.set_backend_established_hook_for_test(Arc::new({
            let commit_attempts = Arc::clone(&commit_attempts);
            move |_, _| {
                commit_attempts.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }));

        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 1).await;
        session_store
            .set_session_state(tmp.path(), &session.id, SessionState::Error)
            .unwrap();
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "late-provider-after-terminal".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let pending = usecase
                    .ctx
                    .sessions
                    .lock()
                    .await
                    .get(&session.id)
                    .is_some_and(RuntimeSessionState::has_pending_provider_establishment);
                if commit_attempts.load(Ordering::SeqCst) >= 1 && !pending {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert_eq!(
            commit_attempts.load(Ordering::SeqCst),
            1,
            "a deterministic lifecycle fence must not enter the transient retry loop"
        );
        assert!(!usecase.provider_session_is_confirmed(&session.id).await);
        let meta = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(meta.state, SessionState::Error);
        assert_eq!(meta.provider_session_generation, 0);
        assert!(meta.agent_session_id.is_none());
        assert!(meta.provider_session_observation_id.is_none());
    }

    #[tokio::test]
    async fn config_modes_persist_without_provider_effect() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        let provider_calls_before = controller.call_kinds_for(&session.id);

        usecase
            .set_permission_mode(&session.id, PermissionMode::Full)
            .await
            .unwrap();
        usecase.set_plan_mode(&session.id, true).await.unwrap();

        let saved = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.permission_mode, "full");
        assert!(saved.plan_mode);
        assert!(event_notifier
            .permission_modes()
            .contains(&(session.id.clone(), "full".to_string())));
        assert_eq!(
            controller.call_kinds_for(&session.id),
            provider_calls_before
        );
    }

    #[tokio::test]
    async fn cross_backend_set_model_changes_an_unstarted_empty_session_without_lifecycle_pause() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        usecase.set_model(&session.id, "codex:gpt-5").await.unwrap();
        let saved = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.backend_id, "codex");
        assert_eq!(saved.selected_model.as_deref(), Some("gpt-5"));
        let model_updates = event_notifier.model_updates();
        assert_eq!(model_updates.len(), 1);
        assert_eq!(model_updates[0].0, session.id);
        assert_eq!(model_updates[0].2, "gpt-5");
        assert!(model_updates[0]
            .1
            .iter()
            .any(|model| model.id == "codex:gpt-5"));
        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session.id)
            .unwrap()
            .is_none());
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .into_iter()
                .filter(|kind| kind == &TestRuntimeCallKind::Close)
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn same_backend_model_is_persisted_now_and_applied_only_inside_turn_execution() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            Arc::new(RecordingAgentNotifier::default()),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();

        usecase
            .set_model(&session.id, "claude:claude-opus-5")
            .await
            .unwrap();

        assert_eq!(
            session_store
                .get_session_meta(tmp.path(), &session.id)
                .unwrap()
                .unwrap()
                .selected_model
                .as_deref(),
            Some("claude-opus-5")
        );
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::SetModel));

        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "apply selected model".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: Some("claude-opus-5".to_string()),
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        let calls = controller.call_kinds_for(&session.id);
        let model_index = calls
            .iter()
            .position(|call| call == &TestRuntimeCallKind::SetModel)
            .expect("turn execution applies the persisted model");
        let start_index = calls
            .iter()
            .position(|call| matches!(call, TestRuntimeCallKind::StartTurn))
            .expect("turn starts");
        assert!(model_index < start_index);
    }

    #[tokio::test]
    async fn cross_backend_set_session_backend_changes_an_unstarted_session() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            Arc::new(RecordingAgentNotifier::default()),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        usecase
            .set_session_backend(&session.id, "codex")
            .await
            .unwrap();

        let saved = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.backend_id, "codex");
        assert_eq!(saved.selected_model.as_deref(), Some("gpt-5"));
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .into_iter()
                .filter(|kind| kind == &TestRuntimeCallKind::Close)
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn cross_backend_set_model_rejects_each_locked_session_state() {
        #[derive(Clone, Copy)]
        enum LockedState {
            Messages,
            AgentSessionId,
            ActiveTurn,
        }

        for locked_state in [
            LockedState::Messages,
            LockedState::AgentSessionId,
            LockedState::ActiveTurn,
        ] {
            {
                let tmp = tempfile::tempdir().unwrap();
                let session_store = Arc::new(build_session_store());
                let (usecase, controller) =
                    build_agent_runtime_usecase_with_controller_and_notifiers(
                        session_store.clone(),
                        tmp.path(),
                        Arc::new(RecordingAgentNotifier::default()),
                        Arc::new(RecordingStatusNotifier::default()),
                    );
                let mut session = create_session_internal_with_attributes(
                    &session_store,
                    tmp.path(),
                    tmp.path().to_string_lossy().as_ref(),
                    Some("claude".to_string()),
                    PermissionMode::Edit,
                    SessionCreationAttributes {
                        selected_model: Some("claude-sonnet-5".to_string()),
                        plan_mode: false,
                        workflow_node_session: false,
                        workflow_node_context: None,
                    },
                )
                .unwrap();
                match locked_state {
                    LockedState::Messages => {
                        session.messages.push(ChatMessage {
                            id: "message-1".to_string(),
                            role: MessageRole::Human,
                            content: "hello".to_string(),
                            thinking: None,
                            activities: None,
                            parts: Some(vec![MessagePart::Text {
                                content: "hello".to_string(),
                                parent_tool_use_id: None,
                            }]),
                            streaming_final_seq: 0,
                            timestamp: 1.0,
                            mentions: None,
                        });
                        session_store
                            .save_full_session_for_restore(tmp.path(), &session)
                            .unwrap();
                    }
                    LockedState::AgentSessionId => {
                        session_store
                            .update_agent_session_id(
                                tmp.path(),
                                &session.id,
                                Some("agent-session".to_string()),
                            )
                            .unwrap();
                    }
                    LockedState::ActiveTurn => {
                        usecase
                            .insert_runtime_state_for_test(&session.id, TurnPhase::Streaming, false)
                            .await;
                    }
                }

                let result = usecase
                    .set_model(&session.id, "codex:gpt-5")
                    .await
                    .map(|_| ());

                assert_eq!(result, Err(AgentRuntimeError::BackendSelectionLocked));
                let saved = session_store
                    .get_session_meta(tmp.path(), &session.id)
                    .unwrap()
                    .unwrap();
                assert_eq!(saved.backend_id, "claude");
                assert_eq!(saved.selected_model.as_deref(), Some("claude-sonnet-5"));
                assert!(!controller
                    .call_kinds_for(&session.id)
                    .contains(&TestRuntimeCallKind::Close));
            }
        }
    }

    #[tokio::test]
    async fn backend_selection_persistence_failure_preserves_runtime_and_previous_selection() {
        for use_set_model in [true, false] {
            let tmp = tempfile::tempdir().unwrap();
            let session_store = Arc::new(build_session_store());
            let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
                session_store.clone(),
                tmp.path(),
                Arc::new(RecordingAgentNotifier::default()),
                Arc::new(RecordingStatusNotifier::default()),
            );
            let session = create_session_internal_with_attributes(
                &session_store,
                tmp.path(),
                tmp.path().to_string_lossy().as_ref(),
                Some("claude".to_string()),
                PermissionMode::Edit,
                SessionCreationAttributes {
                    selected_model: Some("claude-sonnet-5".to_string()),
                    plan_mode: false,
                    workflow_node_session: false,
                    workflow_node_context: None,
                },
            )
            .unwrap();
            usecase
                .start_session(
                    &session.id,
                    StartSessionOptions {
                        permission_mode: PermissionMode::Edit,
                        plan_mode: false,
                    },
                )
                .await
                .unwrap();
            std::fs::remove_file(
                tmp.path()
                    .join("sessions")
                    .join(&session.id)
                    .join("meta.json"),
            )
            .unwrap();

            let result = if use_set_model {
                usecase
                    .set_model(&session.id, "codex:gpt-5")
                    .await
                    .map(|_| ())
            } else {
                usecase
                    .set_session_backend(&session.id, "codex")
                    .await
                    .map(|_| ())
            };

            assert!(result.is_err());
            assert!(usecase.has_live_runtime(&session.id).await);
            let saved = session_store
                .get_session_meta(tmp.path(), &session.id)
                .unwrap()
                .unwrap();
            assert_eq!(saved.backend_id, "claude");
            assert_eq!(saved.selected_model.as_deref(), Some("claude-sonnet-5"));
            assert_eq!(
                controller
                    .call_kinds_for(&session.id)
                    .into_iter()
                    .filter(|kind| kind == &TestRuntimeCallKind::Close)
                    .count(),
                0
            );
        }
    }

    #[tokio::test]
    async fn test_find_permission_request_五一件超の過去ページから解決できる() {
        // Given: a stored session whose permission request is older than the latest 50 messages.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            Some(vec![MessagePart::Permission {
                request: crate::usecase::agent_session::runtime::event_apply::pending_permission_request_from_msg(&PermissionRequestMsg {
                    id: "perm-old".to_string(),
                    tool_use_id: Some("toolu-old".to_string()),
                    tool_name: "Bash".to_string(),
                    kind: PermissionRequestKindMsg::ToolApproval,
                    input: Some(serde_json::json!({"command": "echo old"})),
                    plan: None,
                    allowed_prompts: Vec::new(),
                    questions: Vec::new(),
                    title: Some("Run command".to_string()),
                    display_name: None,
                    description: None,
                    decision_reason: None,
                })
                .unwrap(),
                status: PermissionPartStatus::Allowed,
                answers: None,
                parent_tool_use_id: None,
            }]),
            None,
        )
        .unwrap();
        for index in 0..55 {
            add_message_internal(
                &session_store,
                tmp.path(),
                &session.id,
                MessageRole::Agent,
                &format!("filler {index}"),
                None,
                None,
            )
            .unwrap();
        }

        // When: the permission presentation lookup runs from the latest page.
        let request = usecase
            .find_permission_request(&session.id, "perm-old")
            .await
            .unwrap()
            .expect("permission request");

        // Then: cursor pagination walks back to the older page and returns the stored request.
        assert_eq!(request.id, "perm-old");
        assert_eq!(request.tool_name, "Bash");
        assert_eq!(request.title.as_deref(), Some("Run command"));
    }

    #[tokio::test]
    async fn find_permission_request_returns_in_memory_pending_without_message_part() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        usecase
            .insert_runtime_state_for_test(&session.id, TurnPhase::WaitingPermission, false)
            .await;
        {
            let mut sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get_mut(&session.id).expect("runtime state");
            state.permission_request_cache = Some(permission_request_msg("perm-pending-only"));
            state.restore_stream_buffer_for_test(Vec::new(), Vec::new(), false);
        }

        let request = usecase
            .find_permission_request(&session.id, "perm-pending-only")
            .await
            .unwrap()
            .expect("permission request");

        assert_eq!(request.id, "perm-pending-only");
        assert_eq!(request.tool_name, "Bash");
    }

    #[tokio::test]
    async fn test_permission待機中に終端した後のdeny応答は_busyへ戻さない() {
        // Given: a running turn that has entered WaitingPermission.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store,
                tmp.path(),
            );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(
                    crate::domain::agent_session::entities::PermissionRequest {
                        id: "perm-1".to_string(),
                        tool_use_id: Some("toolu-1".to_string()),
                        parent_tool_use_id: None,
                        tool_name: "Bash".to_string(),
                        body: crate::domain::agent_session::entities::PermissionRequestBody::ToolApproval {
                            input: crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                                r#"{"command":"echo hi"}"#.to_string(),
                            ),
                        },
                        title: None,
                        display_name: None,
                        description: None,
                        decision_reason: None,
                        status: PermissionRequestStatus::Pending,
                    },
                ),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;

        // When: the backend completes the turn before the user denial arrives.
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        let err = usecase
            .respond_permission(
                &session_id,
                PermissionResponse {
                    request_id: "perm-1".to_string(),
                    decision: PermissionResponseDecision::Deny {
                        message: Some("no".to_string()),
                    },
                },
            )
            .await
            .unwrap_err();

        // Then: the late response is rejected and does not move the session back to Streaming.
        assert!(err.to_string().contains("No pending permission request"));
        assert_eq!(usecase.turn_phase(&session_id).await, Some(TurnPhase::Idle));
    }

    #[tokio::test]
    async fn respond_permission_runtime_failure_is_not_blindly_replayed_after_effect_claim() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier,
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-1")),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;
        let before_events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        controller.fail_next_respond_permission();

        let err = usecase
            .respond_permission(
                &session_id,
                PermissionResponse {
                    request_id: "perm-1".to_string(),
                    decision: PermissionResponseDecision::Allow {
                        updated_input: None,
                        answers: None,
                    },
                },
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("permission response failure"));
        assert_eq!(
            session_store
                .load_session_events(tmp.path(), &session_id)
                .unwrap(),
            before_events
        );
        assert!(usecase
            .streaming_parts(&session_id)
            .await
            .iter()
            .any(|part| matches!(
                part,
                MessagePart::Permission {
                    status: PermissionPartStatus::Pending,
                    ..
                }
            )));
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::WaitingPermission)
        );
        let turn_id = session_store
            .get_session_meta(tmp.path(), &session_id)
            .unwrap()
            .and_then(|meta| meta.last_turn_id)
            .expect("active turn id");
        let obligation_id = format!("permission-response:{session_id}:{turn_id}:perm-1");
        let obligation = session_store
            .load_permission_response_obligation(&obligation_id)
            .unwrap()
            .expect("effect reservation");
        assert_eq!(
            obligation,
            crate::domain::local_event::ObligationStateRecord::EffectReserved
        );
        let provider_calls_before = controller
            .call_kinds_for(&session_id)
            .into_iter()
            .filter(|kind| matches!(kind, TestRuntimeCallKind::RespondPermission { .. }))
            .count();

        let retry_error = usecase
            .respond_permission(
                &session_id,
                PermissionResponse {
                    request_id: "perm-1".to_string(),
                    decision: PermissionResponseDecision::Allow {
                        updated_input: None,
                        answers: None,
                    },
                },
            )
            .await
            .unwrap_err();
        assert!(retry_error.to_string().contains("requires reconciliation"));
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .into_iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::RespondPermission { .. }))
                .count(),
            provider_calls_before
        );
    }

    #[tokio::test]
    async fn respond_permission_success_patches_before_persist_event_and_delta() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        usecase.set_workflow_stall_notifier(workflow_stall_notifier.clone());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-1")),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::WaitingPermission).await;
        mark_stall_observation_active_for_test(&usecase, &session_id).await;
        let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        session_store.set_persist_parts_hook_for_test({
            let order = Arc::clone(&order);
            Arc::new(move |_, _, parts| {
                assert!(parts.iter().any(|part| matches!(
                    part,
                    MessagePart::Permission {
                        status: PermissionPartStatus::Allowed,
                        ..
                    }
                )));
                order.lock().unwrap().push("persist");
                Ok(())
            })
        });
        session_store.set_append_event_hook_for_test({
            let order = Arc::clone(&order);
            Arc::new(move |_, event| {
                if matches!(event, AgentSessionEvent::PermissionResolved { .. }) {
                    order.lock().unwrap().push("event");
                }
                Ok(())
            })
        });
        event_notifier.set_streaming_delta_hook({
            let order = Arc::clone(&order);
            Arc::new(move || {
                order.lock().unwrap().push("delta");
            })
        });

        usecase
            .respond_permission(
                &session_id,
                PermissionResponse {
                    request_id: "perm-1".to_string(),
                    decision: PermissionResponseDecision::Allow {
                        updated_input: None,
                        answers: None,
                    },
                },
            )
            .await
            .unwrap();

        wait_for_workflow_stall_cleared_count(&workflow_stall_notifier, 1).await;
        wait_for_stall_clear_count(&event_notifier, 1).await;
        assert_eq!(&*order.lock().unwrap(), &["persist", "event", "delta"]);
        assert_eq!(event_notifier.stall_clears().last(), Some(&session_id));
        assert_eq!(
            workflow_stall_notifier
                .cleared_notifications()
                .last()
                .map(|notification| notification.chat_session_id.as_str()),
            Some(session_id.as_str())
        );
        assert!(event_notifier
            .streaming_deltas()
            .iter()
            .any(|delta| delta.chat_session_id == session_id && delta.snapshot));
        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::Streaming)
        );
    }

    #[tokio::test]
    async fn test_waiting_permissionでtimeout超過してもwatchdogは許可後も非終端signalに留める() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, Some(0))),
            },
        )
        .unwrap();
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::PermissionRequested(
                    crate::domain::agent_session::entities::PermissionRequest {
                        id: "perm-1".to_string(),
                        tool_use_id: Some("toolu-1".to_string()),
                        parent_tool_use_id: None,
                        tool_name: "Bash".to_string(),
                        body: crate::domain::agent_session::entities::PermissionRequestBody::ToolApproval {
                            input: crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                                r#"{"command":"echo hi"}"#.to_string(),
                            ),
                        },
                        title: None,
                        display_name: None,
                        description: None,
                        decision_reason: None,
                        status: PermissionRequestStatus::Pending,
                    },
                ),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session.id, TurnPhase::WaitingPermission).await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(event_notifier.stall_observations().is_empty());

        usecase
            .respond_permission(
                &session.id,
                PermissionResponse {
                    request_id: "perm-1".to_string(),
                    decision: PermissionResponseDecision::Allow {
                        updated_input: None,
                        answers: None,
                    },
                },
            )
            .await
            .unwrap();
        wait_for_turn_phase(&usecase, &session.id, TurnPhase::Streaming).await;
        wait_for_stall_observation_count(&event_notifier, 1).await;
        wait_for_call_count(&controller, &session.id, TestRuntimeCallKind::Reconnect, 1).await;

        let calls = controller.call_kinds_for(&session.id);
        assert!(!calls.contains(&TestRuntimeCallKind::Interrupt));
        assert!(!calls.contains(&TestRuntimeCallKind::Close));
        assert_eq!(
            event_notifier
                .stall_observations()
                .first()
                .map(|payload| payload.turn_phase),
            Some(TurnPhase::Streaming)
        );
    }

    #[tokio::test]
    async fn test_keep_aliveは_last_progress_atを更新する() {
        // Given: a streaming turn whose progress clock has gone stale
        // (e.g. a long-running tool keeps the CLI silent except keep_alive lines).
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store, tmp.path());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Streaming).await;
        let stale_instant = std::time::Instant::now() - Duration::from_secs(3_600);
        {
            let mut sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get_mut(&session_id).unwrap();
            state.restore_runtime_progress_for_test(Some(stale_instant), 1, 0, true);
        }

        // When: the backend emits a keep_alive liveness event.
        controller
            .emit(&session_id, AgentRuntimeEvent::KeepAlive)
            .unwrap();

        // Then: the progress clock is refreshed and the active stall observation is cleared.
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                {
                    let sessions = usecase.ctx.sessions.lock().await;
                    let state = sessions.get(&session_id).unwrap();
                    let last_progress_at = state.last_progress_at().unwrap();
                    if last_progress_at > stale_instant && !state.stall_observation_is_active() {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("keep_alive should refresh last_progress_at");
    }

    #[tokio::test]
    async fn test_workflow_stall_clear失敗時はactive_flagを残し次progressでretryする() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        usecase.set_workflow_stall_notifier(workflow_stall_notifier.clone());
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Streaming).await;
        mark_stall_observation_active_for_test(&usecase, &session_id).await;
        let stale_instant = std::time::Instant::now() - Duration::from_secs(3_600);
        {
            let mut sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get_mut(&session_id).unwrap();
            state.restore_runtime_progress_for_test(Some(stale_instant), 1, 0, true);
        }
        workflow_stall_notifier.fail_next_stall_cleared();

        controller
            .emit(&session_id, AgentRuntimeEvent::KeepAlive)
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                {
                    let sessions = usecase.ctx.sessions.lock().await;
                    let state = sessions.get(&session_id).unwrap();
                    if state
                        .last_progress_at()
                        .is_some_and(|at| at > stale_instant)
                    {
                        assert!(state.stall_observation_is_active());
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("failed clear should still record progress without clearing active flag");
        assert!(workflow_stall_notifier.cleared_notifications().is_empty());
        assert!(event_notifier.stall_clears().is_empty());

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "resumed".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_workflow_stall_cleared_count(&workflow_stall_notifier, 1).await;
        wait_for_stall_clear_count(&event_notifier, 1).await;

        let sessions = usecase.ctx.sessions.lock().await;
        let state = sessions.get(&session_id).unwrap();
        assert!(!state.stall_observation_is_active());
        assert_eq!(event_notifier.stall_clears().last(), Some(&session_id));
        assert_eq!(
            workflow_stall_notifier
                .cleared_notifications()
                .last()
                .map(|notification| notification.chat_session_id.as_str()),
            Some(session_id.as_str())
        );
    }

    #[tokio::test]
    async fn b020_streaming_part_persistence_failure_hides_uncommitted_delta_everywhere() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let agent_message_id = response.agent_message.unwrap().id;
        let failed = Arc::new(AtomicBool::new(false));
        let allow_persist = Arc::new(AtomicBool::new(false));
        session_store.set_persist_parts_hook_for_test({
            let failed = Arc::clone(&failed);
            let allow_persist = Arc::clone(&allow_persist);
            Arc::new(move |_, _, parts| {
                if parts.iter().any(|part| {
                    matches!(part, MessagePart::Text { content, .. } if content == "unsaved")
                }) && !allow_persist.load(Ordering::SeqCst)
                {
                    failed.store(true, Ordering::SeqCst);
                    return Err("injected streaming snapshot failure".to_string());
                }
                Ok(())
            })
        });
        let delta_start = event_notifier.streaming_deltas().len();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "unsaved".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !failed.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the injected commit failure must be observed");

        assert!(
            !usecase
                .streaming_parts(&session_id)
                .await
                .iter()
                .any(|part| {
                    matches!(part, MessagePart::Text { content, .. } if content.contains("unsaved"))
                }),
            "an uncommitted part must not enter the live runtime projection"
        );
        let public_during_failure = usecase
            .get_session(&session_id)
            .await
            .unwrap()
            .expect("the session remains readable while its exact event is retained for retry");
        assert!(
            !public_during_failure
                .session
                .messages
                .iter()
                .any(|message| {
                    message.parts.as_deref().unwrap_or_default().iter().any(|part| {
                    matches!(part, MessagePart::Text { content, .. } if content.contains("unsaved"))
                })
                }),
            "the public read model must not expose an uncommitted part"
        );
        let reloaded_during_failure = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .unwrap();
        assert!(
            !reloaded_during_failure.messages.iter().any(|message| {
                message.parts.as_deref().unwrap_or_default().iter().any(|part| {
                    matches!(part, MessagePart::Text { content, .. } if content.contains("unsaved"))
                })
            }),
            "a fresh durable reload must not expose an uncommitted part"
        );
        assert!(
            !event_notifier.streaming_deltas()[delta_start..]
                .iter()
                .flat_map(|delta| delta.parts.iter())
                .any(|part| {
                    matches!(part, MessagePart::Text { content, .. } if content.contains("unsaved"))
                }),
            "publication must wait for the durable commit"
        );

        allow_persist.store(true, Ordering::SeqCst);
        wait_for_streaming_text(&usecase, &session_id, "unsaved").await;
        assert!(failed.load(Ordering::SeqCst));
        assert!(event_notifier.streaming_deltas()[delta_start..]
            .iter()
            .flat_map(|delta| delta.parts.iter())
            .any(|part| {
                matches!(part, MessagePart::Text { content, .. } if content == "unsaved")
            }));
        let reloaded = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .unwrap();
        assert!(reloaded
            .messages
            .iter()
            .find(|message| message.id == agent_message_id)
            .and_then(|message| message.parts.as_deref())
            .unwrap_or_default()
            .iter()
            .any(|part| {
                matches!(part, MessagePart::Text { content, .. } if content == "unsaved")
            }));

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "saved".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_streaming_text(&usecase, &session_id, "saved").await;
        let reloaded = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .unwrap();
        let persisted_parts = reloaded
            .messages
            .iter()
            .find(|message| message.id == agent_message_id)
            .and_then(|message| message.parts.as_deref())
            .unwrap_or_default();
        assert!(persisted_parts.iter().any(|part| {
            matches!(part, MessagePart::Text { content, .. } if content.contains("saved"))
        }));
        assert!(persisted_parts.iter().any(|part| {
            matches!(part, MessagePart::Text { content, .. } if content.contains("unsaved"))
        }));
    }

    #[derive(Clone, Copy)]
    enum PermissionPartPersistenceFailure {
        Event,
        MessageProjection,
    }

    async fn assert_permission_part_persistence_failure_retries_before_publication(
        failure: PermissionPartPersistenceFailure,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        let agent_message_id = response.agent_message.unwrap().id;
        let failed = Arc::new(AtomicBool::new(false));
        match failure {
            PermissionPartPersistenceFailure::Event => {
                session_store.set_append_event_hook_for_test({
                    let failed = Arc::clone(&failed);
                    Arc::new(move |_, event| {
                        if matches!(event, AgentSessionEvent::PermissionRequested { .. })
                            && !failed.swap(true, Ordering::SeqCst)
                        {
                            return Err("injected permission event failure".to_string());
                        }
                        Ok(())
                    })
                });
            }
            PermissionPartPersistenceFailure::MessageProjection => {
                session_store.set_persist_parts_hook_for_test({
                    let failed = Arc::clone(&failed);
                    Arc::new(move |_, _, parts| {
                        if parts.iter().any(|part| {
                            matches!(
                                part,
                                MessagePart::Permission {
                                    status: PermissionPartStatus::Pending,
                                    ..
                                }
                            )
                        }) && !failed.swap(true, Ordering::SeqCst)
                        {
                            return Err("injected permission projection failure".to_string());
                        }
                        Ok(())
                    })
                });
            }
        }
        let delta_start = event_notifier.streaming_deltas().len();
        let state_start = event_notifier.state_changes().len();

        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PermissionRequested(permission_request("perm-unsaved")),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let live = usecase.get_session(&session_id).await.unwrap().unwrap();
                if failed.load(Ordering::SeqCst)
                    && live.turn_phase == TurnPhase::WaitingPermission
                    && live
                        .pending_permission_request
                        .as_ref()
                        .map(|request| request.id.as_str())
                        == Some("perm-unsaved")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the exact permission request should be retried and published");

        assert_eq!(
            usecase.turn_phase(&session_id).await,
            Some(TurnPhase::WaitingPermission)
        );
        let live = usecase.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(live.turn_phase, TurnPhase::WaitingPermission);
        assert_eq!(
            live.pending_permission_request
                .as_ref()
                .map(|request| request.id.as_str()),
            Some("perm-unsaved")
        );
        assert!(usecase.streaming_parts(&session_id).await.iter().any(|part| {
            matches!(part, MessagePart::Permission { request, .. } if request.id == "perm-unsaved")
        }));
        assert!(event_notifier.streaming_deltas()[delta_start..]
            .iter()
            .flat_map(|delta| delta.parts.iter())
            .any(|part| {
                matches!(part, MessagePart::Permission { request, .. } if request.id == "perm-unsaved")
            }));
        assert!(event_notifier.state_changes()[state_start..]
            .iter()
            .any(|change| {
                change.turn_phase == TurnPhase::WaitingPermission
                    && change
                        .pending_permission_request
                        .as_ref()
                        .map(|request| request.id.as_str())
                        == Some("perm-unsaved")
            }));
        assert!(session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::PermissionRequested { request, .. }
                    if request.id == "perm-unsaved"
            )));
        let reloaded = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .unwrap();
        assert!(reloaded
            .messages
            .iter()
            .find(|message| message.id == agent_message_id)
            .and_then(|message| message.parts.as_deref())
            .unwrap_or_default()
            .iter()
            .any(|part| {
                matches!(part, MessagePart::Permission { request, .. } if request.id == "perm-unsaved")
            }));
    }

    #[tokio::test]
    async fn permission_event_failure_retries_before_publishing_pending_permission() {
        assert_permission_part_persistence_failure_retries_before_publication(
            PermissionPartPersistenceFailure::Event,
        )
        .await;
    }

    #[tokio::test]
    async fn permission_projection_failure_retries_before_publishing_pending_permission() {
        assert_permission_part_persistence_failure_retries_before_publication(
            PermissionPartPersistenceFailure::MessageProjection,
        )
        .await;
    }

    #[tokio::test]
    async fn test_streaming_delta_文字deltaを三三msでcoalesceする() {
        // Given: a started turn with a recording notifier.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();

        // When: the first delta opens the stream, then two more text deltas arrive within the
        // coalescing interval.
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "Hel".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_delta_count(&event_notifier, 1).await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "lo".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "!".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_delta_count(&event_notifier, 2).await;
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Then: the first event is a snapshot and the following two text deltas share one
        // append payload instead of emitting per backend event.
        let deltas = event_notifier.streaming_deltas();
        assert_eq!(deltas.len(), 2);
        assert!(deltas[0].snapshot);
        assert_eq!(deltas[0].seq, 1);
        assert_eq!(
            deltas[0].parts,
            vec![MessagePart::Text {
                content: "Hel".to_string(),
                parent_tool_use_id: None,
            }]
        );
        assert!(!deltas[1].snapshot);
        assert_eq!(deltas[1].seq, 2);
        assert_eq!(
            deltas[1].parts,
            vec![
                MessagePart::Text {
                    content: "lo".to_string(),
                    parent_tool_use_id: None,
                },
                MessagePart::Text {
                    content: "!".to_string(),
                    parent_tool_use_id: None,
                },
            ]
        );
    }

    #[tokio::test]
    async fn test_streaming_delta_emit失敗五連続で通常再送を打ち切りsnapshotフォールバックへ切り替わる(
    ) {
        // Given: a notifier that permanently fails to emit streaming deltas.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        event_notifier.set_streaming_delta_failure(true);
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();

        // When: the first delta keeps failing and another delta arrives after the fallback switch.
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "Hel".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_emit_failure_state(&usecase, &session_id, |failures, _| failures >= 5)
            .await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "lo".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_emit_failure_state(&usecase, &session_id, |_, suppressed| suppressed).await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Then: attempts converge at the failure budget, every attempt is a snapshot resync, and
        // the fallback attempts carry the current full snapshot instead of the frozen retry.
        let deltas = event_notifier.streaming_deltas();
        assert_eq!(deltas.len(), 10);
        assert!(deltas.iter().all(|delta| delta.snapshot && delta.seq == 1));
        assert!(deltas.iter().any(|delta| delta.parts
            == vec![MessagePart::Text {
                content: "Hello".to_string(),
                parent_tool_use_id: None,
            }]));
        assert_eq!(
            usecase
                .stream_emit_failure_state_for_test(&session_id)
                .await,
            Some((10, true))
        );
    }

    #[tokio::test]
    async fn test_streaming_delta_フォールバック後にnotifier回復でsnapshot再同期しdelta配信を再開する(
    ) {
        // Given: streaming emits that fail past the fallback threshold.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        event_notifier.set_streaming_delta_failure(true);
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "Hel".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_emit_failure_state(&usecase, &session_id, |failures, _| failures >= 6)
            .await;

        // When: the notifier recovers while the snapshot fallback is retrying.
        event_notifier.set_streaming_delta_failure(false);
        wait_for_stream_emit_failure_state(&usecase, &session_id, |failures, suppressed| {
            failures == 0 && !suppressed
        })
        .await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "lo".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_last_stream_delta(&event_notifier, |delta| !delta.snapshot).await;

        // Then: the snapshot resync lands with seq 1 and the following delta resumes appends.
        let deltas = event_notifier.streaming_deltas();
        let resync = &deltas[deltas.len() - 2];
        assert!(resync.snapshot);
        assert_eq!(resync.seq, 1);
        assert_eq!(
            resync.parts,
            vec![MessagePart::Text {
                content: "Hel".to_string(),
                parent_tool_use_id: None,
            }]
        );
        let resumed = deltas.last().unwrap();
        assert!(!resumed.snapshot);
        assert_eq!(resumed.seq, 2);
        assert_eq!(
            resumed.parts,
            vec![MessagePart::Text {
                content: "lo".to_string(),
                parent_tool_use_id: None,
            }]
        );
    }

    #[tokio::test]
    async fn b023_terminal_notification_failure_keeps_complete_terminal_live_and_reload_once() {
        // Given: streaming emits that fail until the emit stop threshold is reached.
        let tmp = tempfile::tempdir().unwrap();
        let local_store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
        session_store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        event_notifier.set_streaming_delta_failure(true);
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "hello".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_emit_failure_state(&usecase, &session_id, |_, suppressed| suppressed).await;
        let attempts_after_stop = event_notifier.streaming_deltas().len();

        // When: another delta arrives after emit suppression, then the turn completes.
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: " world".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(event_notifier.streaming_deltas().len(), attempts_after_stop);
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;

        // Then: no further emit attempts happen, the turn completes, and the final message is
        // persisted with every accumulated part.
        assert_eq!(event_notifier.streaming_deltas().len(), attempts_after_stop);
        let live = usecase.get_session(&session_id).await.unwrap().unwrap();
        let restored = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .unwrap();
        let agent_message = restored
            .messages
            .iter()
            .find(|message| message.role == MessageRole::Agent)
            .unwrap();
        assert_eq!(
            agent_message.parts.as_ref().unwrap(),
            &vec![MessagePart::Text {
                content: "hello world".to_string(),
                parent_tool_use_id: None,
            }]
        );
        assert_eq!(live.session.state, SessionState::Done);
        assert_eq!(restored.state, live.session.state);
        assert!(!live.queue_paused);
        assert!(live.pending_queue.is_empty());
        assert!(live.pending_permission_request.is_none());
        let events = session_store
            .load_session_events(tmp.path(), &session_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::TurnCompleted {
                        turn_id: 1,
                        stop_reason: None,
                        ..
                    }
                ))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::FinalPartsRecorded { turn_id: 1, .. }
                ))
                .count(),
            1
        );
        assert_eq!(
            usecase
                .stream_emit_failure_state_for_test(&session_id)
                .await,
            Some((0, false))
        );
    }

    #[tokio::test]
    async fn crash終端snapshotはstreaming_emit完全停止後も回復したnotifierへ着地する() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        event_notifier.set_streaming_delta_failure(true);
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "partial output".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_emit_failure_state(&usecase, &session_id, |_, suppressed| suppressed).await;

        event_notifier.set_streaming_delta_failure(false);
        let delivered_before_crash = event_notifier.delivered_streaming_deltas().len();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::Fatal {
                    message: "CLI process exited".to_string(),
                },
            )
            .unwrap();
        wait_for_error_state_change(&event_notifier, &session_id).await;

        let delivered = event_notifier.delivered_streaming_deltas();
        assert!(delivered[delivered_before_crash..].iter().any(|delta| {
            delta.snapshot
                && delta.parts.iter().any(|part| {
                    matches!(part, MessagePart::Error { content, .. } if content == "CLI process exited")
                })
        }));
    }

    #[tokio::test]
    async fn test_streaming_delta_emit成功で連続失敗カウンタをリセットする() {
        // Given: streaming emits that fail a few times below the fallback threshold.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        event_notifier.set_streaming_delta_failure(true);
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store,
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "Hel".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_emit_failure_state(&usecase, &session_id, |failures, _| failures >= 2)
            .await;

        // When: the notifier recovers before the fallback threshold.
        event_notifier.set_streaming_delta_failure(false);

        // Then: the retry succeeds, the counter resets, and delta delivery continues.
        wait_for_stream_emit_failure_state(&usecase, &session_id, |failures, suppressed| {
            failures == 0 && !suppressed
        })
        .await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "lo".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_last_stream_delta(&event_notifier, |delta| !delta.snapshot).await;
        let resumed = event_notifier.streaming_deltas().pop().unwrap();
        assert_eq!(resumed.seq, 2);
        assert_eq!(
            resumed.parts,
            vec![MessagePart::Text {
                content: "lo".to_string(),
                parent_tool_use_id: None,
            }]
        );
        assert_eq!(
            usecase
                .stream_emit_failure_state_for_test(&session_id)
                .await,
            Some((0, false))
        );
    }

    #[tokio::test]
    async fn test_turn終端後の_trailing_deltaはsnapshot_emitせず確定partsを変更しない() {
        // Given: a completed turn with persisted final parts.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let response = usecase
            .send_message(send_request(tmp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let session_id = response.session.id.clone();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "hello".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_delta_count(&event_notifier, 1).await;
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session_id, TurnPhase::Idle).await;
        let emitted_before_trailing = event_notifier.streaming_deltas().len();

        // When: a backend emits a delayed part after TurnCompleted.
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: " world".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Then: no standalone snapshot is emitted and the terminal winner's parts stay immutable.
        assert_eq!(
            event_notifier.streaming_deltas().len(),
            emitted_before_trailing
        );
        let restored = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .unwrap();
        let agent_message = restored
            .messages
            .iter()
            .find(|message| message.role == MessageRole::Agent)
            .unwrap();
        assert_eq!(
            agent_message.parts.as_ref().unwrap(),
            &vec![MessagePart::Text {
                content: "hello".to_string(),
                parent_tool_use_id: None,
            }]
        );
    }

    #[tokio::test]
    async fn test_start_turn_保存済み会話をreinjectしてpromptへprefixする() {
        // Given: an existing session with prior messages but no backend session id.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Human,
            "remember alpha",
            None,
            None,
        )
        .unwrap();
        add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Agent,
            "alpha acknowledged",
            None,
            None,
        )
        .unwrap();

        // When: the next turn starts through a lazy-open runtime.
        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "what did I ask you to remember?".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        // Then: the backend receives the restore prefix and the session records Reinjected.
        let prompt = controller
            .call_kinds_for(&session.id)
            .into_iter()
            .find_map(|kind| match kind {
                TestRuntimeCallKind::StartTurnPrompt { prompt } => Some(prompt),
                _ => None,
            })
            .expect("start prompt recorded");
        assert!(prompt.contains("releash_restored_conversation"));
        assert!(prompt.contains("remember alpha"));
        assert!(prompt.contains("alpha acknowledged"));
        assert!(prompt.ends_with("what did I ask you to remember?"));
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(
            loaded.session.context_carry,
            Some(ContextCarryState::Reinjected)
        );
    }

    async fn assert_recovery_start_commit_failure_retries_exact_trigger(resume_mismatch: bool) {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = provider_establish_test_session(
            &session_store,
            tmp.path(),
            resume_mismatch.then_some("stored-provider-session"),
        );
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 1).await;

        if !resume_mismatch {
            controller
                .emit(
                    &session.id,
                    AgentRuntimeEvent::SessionEstablished {
                        backend_session_id: "initial-provider-session".to_string(),
                        resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                    },
                )
                .unwrap();
            tokio::time::timeout(Duration::from_secs(1), async {
                while !usecase.provider_session_is_confirmed(&session.id).await {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
        }

        let fail_once = Arc::new(AtomicBool::new(true));
        let attempted_recovery_ids = Arc::new(Mutex::new(Vec::new()));
        session_store.set_append_event_hook_for_test(Arc::new({
            let fail_once = Arc::clone(&fail_once);
            let attempted_recovery_ids = Arc::clone(&attempted_recovery_ids);
            move |_, event| {
                if let AgentSessionEvent::BackendSessionRecoveryStarted { recovery_id, .. } = event
                {
                    attempted_recovery_ids
                        .lock()
                        .unwrap()
                        .push(recovery_id.clone());
                    if fail_once.swap(false, Ordering::SeqCst) {
                        return Err("injected recovery start commit failure".to_string());
                    }
                }
                Ok(())
            }
        }));

        let trigger = if resume_mismatch {
            AgentRuntimeEvent::SessionEstablished {
                backend_session_id: "mismatched-provider-session".to_string(),
                resume: crate::domain::agent_session::gateway::ResumeOutcome::Mismatch {
                    actual: "mismatched-provider-session".to_string(),
                },
            }
        } else {
            AgentRuntimeEvent::BackendSessionCleared
        };
        controller.emit(&session.id, trigger).unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;

        let attempted_recovery_ids = attempted_recovery_ids.lock().unwrap().clone();
        assert!(attempted_recovery_ids.len() >= 2);
        assert!(attempted_recovery_ids
            .iter()
            .all(|recovery_id| recovery_id == &attempted_recovery_ids[0]));
        let recovery_events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .into_iter()
            .filter_map(|event| match event {
                AgentSessionEvent::BackendSessionRecoveryStarted { recovery_id, .. } => {
                    Some(recovery_id)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(recovery_events, vec![attempted_recovery_ids[0].clone()]);
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            2,
            "the retained trigger must not duplicate the provider recovery effect"
        );
    }

    #[tokio::test]
    async fn backend_session_cleared_retries_exact_recovery_trigger_after_begin_commit_failure() {
        assert_recovery_start_commit_failure_retries_exact_trigger(false).await;
    }

    #[tokio::test]
    async fn resume_mismatch_retries_exact_recovery_trigger_after_begin_commit_failure() {
        assert_recovery_start_commit_failure_retries_exact_trigger(true).await;
    }

    #[tokio::test]
    async fn provider_established_retries_same_recovery_completion_after_commit_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 1).await;
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "initial-provider-session".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !usecase.provider_session_is_confirmed(&session.id).await {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        controller
            .emit(&session.id, AgentRuntimeEvent::BackendSessionCleared)
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;
        let recovery_id = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .into_iter()
            .find_map(|event| match event {
                AgentSessionEvent::BackendSessionRecoveryStarted { recovery_id, .. } => {
                    Some(recovery_id)
                }
                _ => None,
            })
            .expect("backend-session-cleared must reserve one recovery identity");

        let completion_attempts = Arc::new(Mutex::new(0_usize));
        session_store.set_append_event_hook_for_test(Arc::new({
            let completion_attempts = Arc::clone(&completion_attempts);
            move |_, event| {
                if matches!(
                    event,
                    AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
                ) {
                    let mut attempts = completion_attempts.lock().unwrap();
                    *attempts += 1;
                    if *attempts == 1 {
                        return Err("injected recovery completion commit failure".to_string());
                    }
                }
                Ok(())
            }
        }));
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "replacement-provider-session".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let meta = session_store
                    .get_session_meta(tmp.path(), &session.id)
                    .unwrap()
                    .unwrap();
                if meta.provider_session_generation == 2
                    && meta.agent_session_id.as_deref() == Some("replacement-provider-session")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        assert_eq!(*completion_attempts.lock().unwrap(), 2);
        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::BackendSessionRecoveryCompleted {
                        recovery_id: actual,
                        ..
                    } if actual == &recovery_id
                ))
                .count(),
            1
        );
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryFailed {
                recovery_id: actual,
                ..
            } if actual == &recovery_id
        )));
        assert!(usecase.provider_session_is_confirmed(&session.id).await);
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            2,
            "retrying the completion commit must not reopen the provider"
        );
    }

    #[tokio::test]
    async fn recovery_failure_commit_retries_same_identity_without_reopening_provider() {
        let tmp = tempfile::tempdir().unwrap();
        let local_store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let session_store = Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
        let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
        session_store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(AgentSessionProjectionCodecV1),
        );
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let session = provider_establish_test_session(&session_store, tmp.path(), None);
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 1).await;
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "initial-provider-session".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !usecase.provider_session_is_confirmed(&session.id).await {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        controller.fail_next_open_session();
        let failure_attempt_ids = Arc::new(Mutex::new(Vec::new()));
        session_store.set_append_event_hook_for_test(Arc::new({
            let failure_attempt_ids = Arc::clone(&failure_attempt_ids);
            move |_, event| {
                if let AgentSessionEvent::BackendSessionRecoveryFailed { recovery_id, .. } = event {
                    let mut ids = failure_attempt_ids.lock().unwrap();
                    ids.push(recovery_id.clone());
                    if ids.len() == 1 {
                        return Err("injected recovery failure commit failure".to_string());
                    }
                }
                Ok(())
            }
        }));
        controller
            .emit(&session.id, AgentRuntimeEvent::BackendSessionCleared)
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if session_store
                    .load_session_events(tmp.path(), &session.id)
                    .unwrap()
                    .iter()
                    .any(|event| {
                        matches!(
                            event,
                            AgentSessionEvent::BackendSessionRecoveryFailed { .. }
                        )
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        let failure_attempt_ids = failure_attempt_ids.lock().unwrap().clone();
        assert_eq!(failure_attempt_ids.len(), 2);
        assert_eq!(failure_attempt_ids[0], failure_attempt_ids[1]);
        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::BackendSessionRecoveryStarted {
                        recovery_id: actual,
                        ..
                    } if actual == &failure_attempt_ids[0]
                ))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::BackendSessionRecoveryFailed {
                        recovery_id: actual,
                        ..
                    } if actual == &failure_attempt_ids[0]
                ))
                .count(),
            1
        );
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            2,
            "the recovery provider effect must run exactly once"
        );
        assert_eq!(
            session_store
                .get_session_meta(tmp.path(), &session.id)
                .unwrap()
                .unwrap()
                .state,
            SessionState::Error
        );
    }

    #[tokio::test]
    async fn test_resume_mismatch_進行中turnをrequeueしてreinjectで再開する() {
        // Given: a session that tries to resume an old backend session.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("old-backend-session".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        add_message_internal(
            &session_store,
            tmp.path(),
            &session.id,
            MessageRole::Human,
            "remember alpha",
            None,
            None,
        )
        .unwrap();

        // When: the backend reports that the resumed id does not match the actual thread.
        let editor_context = AgentEditorContext {
            active_editor_path: Some("src/main.rs".to_string()),
            open_editor_paths: vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
            selection: Some(AgentEditorSelection {
                file_path: "src/main.rs".to_string(),
                start_line: 4,
                end_line: 9,
            }),
        };
        let images = vec![ImageAttachment {
            data: "iVBORw==".to_string(),
            media_type: "image/png".to_string(),
        }];
        let mentions = vec![crate::domain::code::MentionReference {
            file_path: "src/lib.rs".to_string(),
            start_line: Some(12),
            end_line: Some(18),
        }];
        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "continue".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: Some(images.clone()),
                mentions: Some(mentions.clone()),
                editor_context: Some(editor_context.clone()),
            })
            .await
            .unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 1).await;
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "actual-backend-session".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::Mismatch {
                        actual: "actual-backend-session".to_string(),
                    },
                },
            )
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurnPrompt { .. }))
                .count(),
            1,
            "the retry must remain queued until the new backend session is established"
        );
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "fresh-backend-session".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 2).await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Then: the retry prompt is reinjected, the stale backend id is cleared, and the
        // mismatched runtime was closed before reopening.
        let prompts = controller
            .call_kinds_for(&session.id)
            .into_iter()
            .filter_map(|kind| match kind {
                TestRuntimeCallKind::StartTurnPrompt { prompt } => Some(prompt),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(prompts[0], "continue");
        assert!(prompts[1].contains("releash_restored_conversation"));
        assert!(prompts[1].contains("remember alpha"));
        assert!(prompts[1].ends_with("continue"));
        assert!(controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Close));
        let editor_contexts = controller
            .call_kinds_for(&session.id)
            .into_iter()
            .filter_map(|kind| match kind {
                TestRuntimeCallKind::StartTurnEditorContext { editor_context } => editor_context,
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(editor_contexts.len(), 2);
        assert_eq!(
            editor_contexts[0],
            EditorContext::from(editor_context.clone())
        );
        assert_eq!(editor_contexts[1], EditorContext::from(editor_context));
        let turn_images = controller
            .call_kinds_for(&session.id)
            .into_iter()
            .filter_map(|kind| match kind {
                TestRuntimeCallKind::StartTurnImages { images } => Some(images),
                _ => None,
            })
            .collect::<Vec<_>>();
        let expected_images = images
            .into_iter()
            .map(|image| AttachmentPayload {
                data: image.data,
                media_type: image.media_type,
            })
            .collect::<Vec<_>>();
        assert_eq!(turn_images, vec![expected_images.clone(), expected_images]);
        let system_prompts = controller
            .call_kinds_for(&session.id)
            .into_iter()
            .filter_map(|kind| match kind {
                TestRuntimeCallKind::StartTurnSystemPrompt { system_prompt } => Some(system_prompt),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(system_prompts.len(), 2);
        assert_eq!(system_prompts[0], system_prompts[1]);
        assert!(system_prompts[0]
            .as_deref()
            .is_some_and(|prompt| prompt.contains("src/lib.rs")));
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(
            loaded.session.agent_session_id.as_deref(),
            Some("fresh-backend-session")
        );
        assert_eq!(
            loaded.session.context_carry,
            Some(ContextCarryState::Reinjected)
        );
        assert!(loaded.session.messages.iter().any(|message| {
            message.parts.as_deref().is_some_and(|parts| {
                parts.iter().any(|part| matches!(
                    part,
                    MessagePart::SystemNotification {
                        notification_type: crate::usecase::agent_session::session::SystemNotificationType::SessionRecovery,
                        label,
                        ..
                    } if label == "backend セッションを作り直したため文脈は引き継がれません"
                ))
            })
        }));

        let recovery_events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .into_iter()
            .filter(|event| {
                matches!(
                    event,
                    AgentSessionEvent::BackendSessionRecoveryStarted { .. }
                        | AgentSessionEvent::SessionConfigurationReactivated { .. }
                        | AgentSessionEvent::SessionGoalReactivated { .. }
                        | AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
                )
            })
            .collect::<Vec<_>>();
        let retried_mentions = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .into_iter()
            .filter_map(|event| match event {
                AgentSessionEvent::TurnStarted { prompt, .. } if prompt.content == "continue" => {
                    Some(prompt.mentions)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let expected_mentions = mentions;
        assert_eq!(
            retried_mentions,
            vec![expected_mentions.clone(), expected_mentions]
        );
        assert_eq!(recovery_events.len(), 4);
        let recovery_id = match &recovery_events[0] {
            AgentSessionEvent::BackendSessionRecoveryStarted { recovery_id, .. } => {
                recovery_id.clone()
            }
            event => panic!("unexpected recovery event: {event:?}"),
        };
        assert!(matches!(
            &recovery_events[1],
            AgentSessionEvent::SessionConfigurationReactivated {
                recovery_id: actual,
                provider_session_generation: 1,
                ..
            } if actual == &recovery_id
        ));
        assert!(matches!(
            &recovery_events[2],
            AgentSessionEvent::SessionGoalReactivated {
                recovery_id: actual,
                outcome: crate::usecase::agent_session::event_log::GoalReactivationOutcome::NoCurrentGoal,
                provider_session_generation: 1,
                ..
            } if actual == &recovery_id
        ));
        assert!(matches!(
            &recovery_events[3],
            AgentSessionEvent::BackendSessionRecoveryCompleted {
                recovery_id: actual,
                provider_session_generation: 1,
                ..
            } if actual == &recovery_id
        ));
    }

    #[test]
    fn completed_recovery_restore_policy_is_identical_after_runtime_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let original_store = Arc::new(build_session_store());
        let session = create_session_internal_with_attributes(
            &original_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        add_message_internal(
            &original_store,
            tmp.path(),
            &session.id,
            MessageRole::Human,
            "remember durable context",
            None,
            None,
        )
        .unwrap();
        original_store
            .begin_backend_session_recovery(
                tmp.path(),
                &session.id,
                "durable-recovery",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        original_store
            .complete_backend_session_recovery(
                tmp.path(),
                &session.id,
                "durable-recovery",
                0,
                "fresh-provider-session".to_string(),
            )
            .unwrap();

        let (running_usecase, _) =
            build_agent_runtime_usecase_with_controller(original_store.clone(), tmp.path());
        let without_restart = context_restore_policy_for_turn(
            &running_usecase.ctx,
            &session.id,
            "next-agent-message",
            true,
        )
        .unwrap();
        let prompt_without_restart =
            apply_restore_prompt_prefix("continue".to_string(), &without_restart.plan);
        let carry_without_restart = original_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap()
            .context_carry;
        drop(running_usecase);
        drop(original_store);

        let reopened_store = Arc::new(build_session_store());
        let (restarted_usecase, _) =
            build_agent_runtime_usecase_with_controller(reopened_store.clone(), tmp.path());
        let after_restart = context_restore_policy_for_turn(
            &restarted_usecase.ctx,
            &session.id,
            "next-agent-message",
            false,
        )
        .unwrap();
        let prompt_after_restart =
            apply_restore_prompt_prefix("continue".to_string(), &after_restart.plan);
        let meta_after_restart = reopened_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();

        assert!(without_restart.recovery_restore_required);
        assert!(after_restart.recovery_restore_required);
        assert_eq!(prompt_after_restart, prompt_without_restart);
        assert!(prompt_after_restart.contains("releash_restored_conversation"));
        assert!(prompt_after_restart.contains("remember durable context"));
        assert_eq!(meta_after_restart.context_carry, carry_without_restart);
        assert_eq!(meta_after_restart.context_reinjection_generation, Some(1));
    }

    #[tokio::test]
    async fn test_backend_session_clearedは新規sessionでturnを再開する() {
        // Given: a session that was previously resumed.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("backend-session".to_string());
        session.context_carry = Some(ContextCarryState::Resumed);
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "continue".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        // When: the backend reports that its resumable session was cleared.
        controller
            .emit(&session.id, AgentRuntimeEvent::BackendSessionCleared)
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "replacement-session".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 2).await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Then: the dead backend id is replaced and the original turn is retried.
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(
            loaded.session.agent_session_id.as_deref(),
            Some("replacement-session")
        );
        assert_eq!(
            loaded.session.context_carry,
            Some(ContextCarryState::Failed)
        );
    }

    #[tokio::test]
    async fn recovery_reopens_with_latest_persisted_configuration_and_generation_two() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller, workspace_query) =
            build_agent_runtime_usecase_with_controller_and_workspace_query(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        let normal_session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes::default(),
        )
        .unwrap();
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "initial-thread".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let generation = session_store
                    .get_session_meta(tmp.path(), &session.id)
                    .unwrap()
                    .unwrap()
                    .provider_session_generation;
                if generation == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            session_store
                .get_session_meta(tmp.path(), &session.id)
                .unwrap()
                .unwrap()
                .provider_session_generation,
            1
        );

        session_store
            .update_backend_selection(
                tmp.path(),
                &session.id,
                "claude".to_string(),
                Some("claude-4-opus".to_string()),
            )
            .unwrap();
        session_store
            .update_permission_mode(tmp.path(), &session.id, PermissionMode::FULL)
            .unwrap();
        session_store
            .update_plan_mode(tmp.path(), &session.id, true)
            .unwrap();
        let before_recovery = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap()
            .to_summary();
        let normal_before_recovery = session_store
            .get_session_meta(tmp.path(), &normal_session.id)
            .unwrap()
            .unwrap()
            .to_summary();

        controller
            .emit(&session.id, AgentRuntimeEvent::BackendSessionCleared)
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;

        let opens = controller
            .call_kinds_for(&session.id)
            .into_iter()
            .filter_map(|kind| match kind {
                TestRuntimeCallKind::OpenSession {
                    resume,
                    model,
                    permission_mode,
                    plan_mode,
                    ..
                } => Some((resume, model, permission_mode, plan_mode)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(opens.len(), 2);
        assert_eq!(opens[1].0, None);
        assert_eq!(opens[1].1, "claude-4-opus");
        assert_eq!(opens[1].2, PermissionMode::Full);
        assert!(opens[1].3);

        let recovering_publication = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap()
            .recovery_publication_snapshot
            .expect("recovery publishes the last stable summary")
            .summary;
        workspace_query.replace_session_summaries(
            vec![recovering_publication, normal_before_recovery.clone()],
            Vec::new(),
        );
        let worktree_path = tmp.path().to_string_lossy().to_string();
        let tauri_sessions = usecase.list_sessions(&worktree_path).await.unwrap();
        let tauri_recovering = tauri_sessions
            .iter()
            .find(|summary| summary.id == session.id)
            .unwrap();
        assert_eq!(tauri_recovering.state, SessionState::Active);
        assert_eq!(tauri_recovering.updated_at, before_recovery.updated_at);
        let tauri_normal = tauri_sessions
            .iter()
            .find(|summary| summary.id == normal_session.id)
            .unwrap();
        assert_eq!(tauri_normal.state, normal_before_recovery.state);
        assert_eq!(tauri_normal.updated_at, normal_before_recovery.updated_at);

        let events_during_recovery = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        let recovery_id = events_during_recovery
            .iter()
            .find_map(|event| match event {
                AgentSessionEvent::BackendSessionRecoveryStarted {
                    recovery_id,
                    old_provider_session_generation: 1,
                    ..
                } => Some(recovery_id.clone()),
                _ => None,
            })
            .expect("recovery starts from the established generation");

        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "fresh-thread".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let generation = session_store
                    .get_session_meta(tmp.path(), &session.id)
                    .unwrap()
                    .unwrap()
                    .provider_session_generation;
                if generation == 2 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        let recovered_meta = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(recovered_meta.backend_id, "claude");
        assert_eq!(
            recovered_meta.selected_model.as_deref(),
            Some("claude-4-opus")
        );
        assert_eq!(recovered_meta.permission_mode, PermissionMode::FULL);
        assert!(recovered_meta.plan_mode);
        assert_eq!(recovered_meta.provider_session_generation, 2);
        let recovered_events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert!(recovered_events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::SessionConfigurationReactivated {
                recovery_id: actual,
                provider_session_generation: 2,
                ..
            } if actual == &recovery_id
        )));
        assert!(recovered_events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::SessionGoalReactivated {
                recovery_id: actual,
                provider_session_generation: 2,
                ..
            } if actual == &recovery_id
        )));
        assert!(recovered_events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryCompleted {
                recovery_id: actual,
                provider_session_generation: 2,
                ..
            } if actual == &recovery_id
        )));
    }

    #[tokio::test]
    async fn test_codex_resume失敗はfresh_sessionで復活しdead_threadを再利用しない() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller, workspace_query) =
            build_agent_runtime_usecase_with_controller_and_workspace_query(
                session_store.clone(),
                tmp.path(),
            );
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("dead-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        let unaffected_session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes::default(),
        )
        .unwrap();
        controller.fail_next_resume_open();

        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "recover this turn".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        wait_for_open_count(&controller, &session.id, 2).await;
        let resumes = controller
            .call_kinds_for(&session.id)
            .into_iter()
            .filter_map(|kind| match kind {
                TestRuntimeCallKind::OpenSession { resume, .. } => Some(resume),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            resumes,
            vec![Some("dead-thread".to_string()), None],
            "recovery must clear resume metadata before opening the replacement session"
        );
        assert!(session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryStarted {
                    old_provider_session_generation: 0,
                    reason: BackendSessionRecoveryReason::BackendSessionLost,
                    ..
                }
            )));
        assert!(!session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
            )));

        let recovering_publication = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap()
            .recovery_publication_snapshot
            .expect("recovery publishes the last stable summary")
            .summary;
        workspace_query.replace_session_summaries(
            vec![recovering_publication, unaffected_session.to_summary()],
            Vec::new(),
        );
        let worktree_path = tmp.path().to_string_lossy().to_string();
        let listed_during_recovery = tokio::time::timeout(
            Duration::from_secs(1),
            usecase.list_sessions(&worktree_path),
        )
        .await
        .expect("another session's list must not wait for recovery establishment")
        .unwrap();
        assert!(listed_during_recovery
            .iter()
            .any(|summary| summary.id == unaffected_session.id));
        assert_eq!(
            listed_during_recovery
                .iter()
                .find(|summary| summary.id == session.id)
                .expect("recovering session keeps its previously published summary")
                .agent_session_id
                .as_deref(),
            Some("dead-thread"),
            "the recovering session must not publish its cleared resume metadata"
        );

        let config_usecase = Arc::clone(&usecase);
        let config_session_id = session.id.clone();
        let config_update = tokio::spawn(async move {
            config_usecase
                .set_model(&config_session_id, "codex:gpt-5")
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !config_update.is_finished(),
            "configuration changes must remain blocked until recovery completes"
        );

        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "fresh-thread".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        config_update.await.unwrap().unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 1).await;
        let recovered_publication = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap()
            .to_summary();
        workspace_query.replace_session_summaries(
            vec![recovered_publication, unaffected_session.to_summary()],
            Vec::new(),
        );
        let listed = usecase.list_sessions(&worktree_path).await.unwrap();
        let listed_session = listed
            .iter()
            .find(|summary| summary.id == session.id)
            .expect("recovered session remains listed");
        assert_eq!(
            listed_session.agent_session_id.as_deref(),
            Some("fresh-thread")
        );
        tokio::time::sleep(Duration::from_millis(20)).await;

        let recovered = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(
            recovered.session.agent_session_id.as_deref(),
            Some("fresh-thread")
        );
        let meta = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(meta.provider_session_generation, 1);
        assert!(recovered.session.messages.iter().any(|message| {
            message.parts.as_deref().is_some_and(|parts| {
                parts.iter().any(|part| matches!(
                    part,
                    MessagePart::SystemNotification {
                        notification_type: crate::usecase::agent_session::session::SystemNotificationType::SessionRecovery,
                        ..
                    }
                ))
            })
        }));

        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session.id, TurnPhase::Idle).await;
        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "follow up".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 2).await;
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::OpenSession { .. }))
                .count(),
            2,
            "the recovered live runtime must not reopen the dead thread"
        );
    }

    #[tokio::test]
    async fn recovery_completion_without_pending_turn_publishes_notice_immediately() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("dead-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        controller.fail_next_resume_open();

        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "fresh-thread".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;

        for _ in 0..2 {
            let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
            assert_eq!(
                loaded
                    .session
                    .messages
                    .iter()
                    .flat_map(|message| message.parts.as_deref().unwrap_or_default())
                    .filter(|part| matches!(
                        part,
                        MessagePart::SystemNotification {
                            notification_type: SystemNotificationType::SessionRecovery,
                            ..
                        }
                    ))
                    .count(),
                1
            );
        }
        assert!(!controller
            .call_kinds_for(&session.id)
            .iter()
            .any(|kind| matches!(kind, TestRuntimeCallKind::StartTurn)));
    }

    #[derive(Clone, Copy, Debug)]
    enum B036CrashBoundary {
        AfterRecoveryStart,
        AfterExternalEffect,
        AfterCompletion,
        BeforeMessagePublication,
    }

    #[tokio::test]
    async fn b036_recovery_crash_boundaries_preserve_identity_and_limit_effect_and_message_to_one()
    {
        for boundary in [
            B036CrashBoundary::AfterRecoveryStart,
            B036CrashBoundary::AfterExternalEffect,
            B036CrashBoundary::AfterCompletion,
            B036CrashBoundary::BeforeMessagePublication,
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let local_store =
                LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                    .unwrap();
            let session_store =
                Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
            let repository: Arc<dyn LocalEventTransactionRepository> = local_store.clone();
            session_store.set_local_event_repository(
                repository,
                local_store.installation_id().to_string(),
                Arc::new(AgentSessionProjectionCodecV1),
            );
            let session =
                provider_establish_test_session(&session_store, tmp.path(), Some("dead-provider"));
            let recovery_id = format!("b036-recovery-{boundary:?}");
            let old_generation = session_store
                .get_session_meta(tmp.path(), &session.id)
                .unwrap()
                .unwrap()
                .provider_session_generation;

            let external_effect_count = match boundary {
                B036CrashBoundary::AfterRecoveryStart => {
                    session_store
                        .begin_backend_session_recovery(
                            tmp.path(),
                            &session.id,
                            &recovery_id,
                            BackendSessionRecoveryReason::BackendSessionLost,
                        )
                        .unwrap();
                    0
                }
                B036CrashBoundary::AfterExternalEffect => {
                    let (usecase, controller) =
                        build_agent_runtime_usecase_with_controller_and_spawner(
                            session_store.clone(),
                            tmp.path(),
                            Arc::new(DroppingSpawner),
                        );
                    controller.fail_next_open_session();
                    assert!(recover_backend_session_with_identity(
                        &usecase.ctx,
                        &session.id,
                        BackendSessionRecoveryReason::BackendSessionLost,
                        recovery_id.clone(),
                    )
                    .await
                    .is_ok());
                    let count = controller
                        .call_kinds_for(&session.id)
                        .iter()
                        .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                        .count();
                    assert_eq!(count, 1, "{boundary:?} must cross the effect port once");
                    count
                }
                B036CrashBoundary::AfterCompletion
                | B036CrashBoundary::BeforeMessagePublication => {
                    let (usecase, controller) = build_agent_runtime_usecase_with_controller(
                        session_store.clone(),
                        tmp.path(),
                    );
                    recover_backend_session_with_identity(
                        &usecase.ctx,
                        &session.id,
                        BackendSessionRecoveryReason::BackendSessionLost,
                        recovery_id.clone(),
                    )
                    .await
                    .unwrap();
                    let count = controller
                        .call_kinds_for(&session.id)
                        .iter()
                        .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                        .count();
                    assert_eq!(count, 1, "{boundary:?} must cross the effect port once");
                    session_store
                        .complete_backend_session_recovery(
                            tmp.path(),
                            &session.id,
                            &recovery_id,
                            old_generation,
                            "replacement-provider".to_string(),
                        )
                        .unwrap();
                    if matches!(boundary, B036CrashBoundary::BeforeMessagePublication) {
                        local_store.fault_injector().arm_fail_before_begin();
                        assert!(
                            reconcile_pending_recovery_message(&usecase.ctx, &session.id)
                                .await
                                .is_err()
                        );
                        assert!(session_store
                            .get_session_meta(tmp.path(), &session.id)
                            .unwrap()
                            .unwrap()
                            .pending_recovery_message
                            .is_some());
                    }
                    controller.close_event_streams_for_test(&session.id);
                    count
                }
            };

            let before_restart_events = session_store
                .load_session_events(tmp.path(), &session.id)
                .unwrap();
            assert_eq!(
                before_restart_events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        AgentSessionEvent::BackendSessionRecoveryStarted {
                            recovery_id: actual,
                            ..
                        } if actual == &recovery_id
                    ))
                    .count(),
                1,
                "{boundary:?} must retain exactly one recovery identity"
            );
            let completed_before_restart = matches!(
                boundary,
                B036CrashBoundary::AfterCompletion | B036CrashBoundary::BeforeMessagePublication
            );
            assert_eq!(
                before_restart_events.iter().any(|event| matches!(
                    event,
                    AgentSessionEvent::BackendSessionRecoveryCompleted {
                        recovery_id: actual,
                        ..
                    } if actual == &recovery_id
                )),
                completed_before_restart
            );
            drop(session_store);
            drop(local_store);
            tokio::task::yield_now().await;

            let reopened_store =
                LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                    .unwrap();
            let reopened_session_store =
                Arc::new(SessionStore::new(Arc::new(FileSessionStorage::default())));
            let repository: Arc<dyn LocalEventTransactionRepository> = reopened_store.clone();
            reopened_session_store.set_local_event_repository(
                repository,
                reopened_store.installation_id().to_string(),
                Arc::new(AgentSessionProjectionCodecV1),
            );
            let (restarted, restart_controller) = build_agent_runtime_usecase_with_controller(
                reopened_session_store.clone(),
                tmp.path(),
            );
            let first = restarted.get_session(&session.id).await.unwrap().unwrap();
            let second = restarted.get_session(&session.id).await.unwrap().unwrap();
            let recovery_message_count = |response: &GetSessionResponse| {
                response
                    .session
                    .messages
                    .iter()
                    .filter(|message| {
                        message.parts.as_deref().is_some_and(|parts| {
                            parts.iter().any(|part| {
                                matches!(
                                    part,
                                    MessagePart::SystemNotification {
                                        notification_type: SystemNotificationType::SessionRecovery,
                                        ..
                                    }
                                ) || matches!(
                                    part,
                                    MessagePart::Error { content, .. }
                                        if content.starts_with("backend session recovery failed:")
                                )
                            })
                        })
                    })
                    .count()
            };
            assert_eq!(recovery_message_count(&first), 1, "{boundary:?}");
            assert_eq!(recovery_message_count(&second), 1, "{boundary:?}");
            assert_eq!(
                external_effect_count
                    + restart_controller
                        .call_kinds_for(&session.id)
                        .iter()
                        .filter(|call| matches!(call, TestRuntimeCallKind::OpenSession { .. }))
                        .count(),
                external_effect_count,
                "restart must not repeat the recovery provider effect at {boundary:?}"
            );
            assert!(external_effect_count <= 1);

            let after_restart_events = reopened_session_store
                .load_session_events(tmp.path(), &session.id)
                .unwrap();
            let completed = after_restart_events
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        AgentSessionEvent::BackendSessionRecoveryCompleted {
                            recovery_id: actual,
                            ..
                        } if actual == &recovery_id
                    )
                })
                .count();
            let failed = after_restart_events
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        AgentSessionEvent::BackendSessionRecoveryFailed {
                            recovery_id: actual,
                            ..
                        } if actual == &recovery_id
                    )
                })
                .count();
            assert_eq!(completed + failed, 1, "{boundary:?} must be fully resolved");
            assert_eq!(completed, usize::from(completed_before_restart));
            assert!(reopened_session_store
                .get_session_meta(tmp.path(), &session.id)
                .unwrap()
                .unwrap()
                .pending_recovery_message
                .is_none());
        }
    }

    #[tokio::test]
    async fn completed_recovery_notice_is_restored_once_before_the_next_turn_after_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let original_store = Arc::new(build_session_store());
        let session = create_session_internal_with_attributes(
            &original_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        original_store
            .begin_backend_session_recovery(
                tmp.path(),
                &session.id,
                "completed-before-restart",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        original_store
            .complete_backend_session_recovery(
                tmp.path(),
                &session.id,
                "completed-before-restart",
                0,
                "fresh-thread".to_string(),
            )
            .unwrap();
        let before_restart = original_store
            .load_full_session_for_restore(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert!(before_restart.messages.is_empty());
        assert!(original_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap()
            .pending_recovery_message
            .is_some());
        drop(original_store);

        let reopened_store = Arc::new(build_session_store());
        let (usecase, _) =
            build_agent_runtime_usecase_with_controller(reopened_store.clone(), tmp.path());
        let recovered = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(
            recovered
                .session
                .messages
                .iter()
                .flat_map(|message| message.parts.as_deref().unwrap_or_default())
                .filter(|part| matches!(
                    part,
                    MessagePart::SystemNotification {
                        notification_type: SystemNotificationType::SessionRecovery,
                        ..
                    }
                ))
                .count(),
            1
        );

        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "next turn after restart".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        let persisted = reopened_store
            .load_full_session_for_restore(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        let notice_index = persisted
            .messages
            .iter()
            .position(|message| {
                message.parts.as_deref().is_some_and(|parts| {
                    parts.iter().any(|part| {
                        matches!(
                            part,
                            MessagePart::SystemNotification {
                                notification_type: SystemNotificationType::SessionRecovery,
                                ..
                            }
                        )
                    })
                })
            })
            .unwrap();
        let next_turn_index = persisted
            .messages
            .iter()
            .position(|message| message.content == "next turn after restart")
            .unwrap();
        assert!(notice_index < next_turn_index);
        assert_eq!(
            persisted
                .messages
                .iter()
                .flat_map(|message| message.parts.as_deref().unwrap_or_default())
                .filter(|part| matches!(
                    part,
                    MessagePart::SystemNotification {
                        notification_type: SystemNotificationType::SessionRecovery,
                        ..
                    }
                ))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn recovery_notice_survives_retried_turn_start_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("old-session".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "retry me".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        controller
            .emit(&session.id, AgentRuntimeEvent::BackendSessionCleared)
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;
        controller.fail_next_start_turn();
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "fresh-session".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 2).await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.session.state, SessionState::Error);
        assert!(loaded.session.messages.iter().any(|message| {
            message.parts.as_deref().is_some_and(|parts| {
                parts.iter().any(|part| {
                    matches!(
                        part,
                        MessagePart::SystemNotification {
                            notification_type: SystemNotificationType::SessionRecovery,
                            ..
                        }
                    )
                })
            })
        }));
    }

    #[tokio::test]
    async fn failed_recovery_error_part_is_reconciled_once_after_a_write_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("dead-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        let fail_error_once = Arc::new(AtomicBool::new(true));
        session_store.set_persist_parts_hook_for_test(Arc::new({
            let fail_error_once = fail_error_once.clone();
            move |_, _, parts| {
                if parts
                    .iter()
                    .any(|part| matches!(part, MessagePart::Error { .. }))
                    && fail_error_once.swap(false, Ordering::SeqCst)
                {
                    return Err("injected recovery error persistence failure".to_string());
                }
                Ok(())
            }
        }));
        controller.fail_next_resume_open();
        controller.fail_next_open();

        let result = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "recover".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await;
        assert!(
            result.is_ok(),
            "the send was durably accepted before backend recovery failed"
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let meta = session_store
                    .get_session_meta(tmp.path(), &session.id)
                    .unwrap()
                    .unwrap();
                if !fail_error_once.load(Ordering::SeqCst)
                    && meta.state == SessionState::Error
                    && meta.pending_recovery_message.is_some()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the detached recovery failure must settle before reconciliation");
        let before_reconcile = session_store
            .load_full_session_for_restore(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(before_reconcile.state, SessionState::Error);
        assert!(!before_reconcile.messages.iter().any(|message| {
            message.parts.as_deref().is_some_and(|parts| {
                parts
                    .iter()
                    .any(|part| matches!(part, MessagePart::Error { .. }))
            })
        }));
        assert!(session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap()
            .pending_recovery_message
            .is_some());

        for _ in 0..2 {
            let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
            assert_eq!(loaded.session.state, SessionState::Error);
            assert_eq!(
                loaded
                    .session
                    .messages
                    .iter()
                    .flat_map(|message| message.parts.as_deref().unwrap_or_default())
                    .filter(|part| matches!(
                        part,
                        MessagePart::Error { content, .. }
                            if content.contains("injected test open failure")
                    ))
                    .count(),
                1
            );
        }
        assert!(session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap()
            .pending_recovery_message
            .is_none());
        assert!(session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryFailed { .. }
            )));
    }

    #[tokio::test]
    async fn startup_recovery_begin_failure_does_not_start_cleanup_effects() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        session_store.set_append_event_hook_for_test(Arc::new(|_, event| {
            if matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryStarted { .. }
            ) {
                return Err("injected recovery begin failure".to_string());
            }
            Ok(())
        }));
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("dead-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        controller.fail_next_resume_open();

        let result = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "startup recovery".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await;
        assert!(result.is_err());
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Interrupt));
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Close));
        assert!(event_notifier.streaming_deltas().is_empty());
        assert!(!session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryFailed { .. }
            )));
    }

    #[tokio::test]
    async fn live_recovery_begin_failure_preserves_runtime_and_state_without_provider_effect() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            Arc::new(RecordingStatusNotifier::default()),
        );
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("live-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "live recovery".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        wait_for_turn_phase(&usecase, &session.id, TurnPhase::Streaming).await;
        session_store.set_append_event_hook_for_test(Arc::new(|_, event| {
            if matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryStarted { .. }
            ) {
                return Err("injected recovery begin failure".to_string());
            }
            Ok(())
        }));

        controller
            .emit(&session.id, AgentRuntimeEvent::BackendSessionCleared)
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;

        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.turn_phase, TurnPhase::Streaming);
        assert_eq!(loaded.session.state, SessionState::Active);
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Interrupt));
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Close));
        assert!(event_notifier.streaming_deltas().is_empty());
    }

    #[tokio::test]
    async fn recovery_completion_commit_failure_does_not_publish_a_false_error() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        session_store.set_append_event_hook_for_test(Arc::new(|_, event| {
            if matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
            ) {
                return Err("injected completion commit failure".to_string());
            }
            Ok(())
        }));
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("dead-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        controller.fail_next_resume_open();
        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "recover".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "fresh-thread".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;

        let loaded = session_store
            .load_full_session_for_restore(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.state, SessionState::Active);
        assert_eq!(loaded.context_carry, Some(ContextCarryState::Failed));
        assert!(loaded.agent_session_id.is_none());
        assert!(!loaded.messages.iter().any(|message| {
            message.role == MessageRole::Agent
                && message.parts.as_deref().is_some_and(|parts| {
                    parts
                        .iter()
                        .any(|part| matches!(part, MessagePart::Error { .. }))
                })
        }));
        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert!(matches!(
            TurnEventLog::from_events(events.clone())
                .project()
                .backend_recovery,
            Some(BackendSessionRecoveryProjection::Recovering { .. })
        ));
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
                | AgentSessionEvent::BackendSessionRecoveryFailed { .. }
        )));
    }

    #[tokio::test]
    async fn recovery_notice_persistence_failure_does_not_demote_completed_recovery() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller, workspace_query) =
            build_agent_runtime_usecase_with_controller_and_workspace_query(
                session_store.clone(),
                tmp.path(),
            );
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("dead-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        controller.fail_next_resume_open();
        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "recover".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;
        let fail_notice_once = Arc::new(AtomicBool::new(true));
        session_store.set_append_message_hook_for_test(Arc::new({
            let fail_notice_once = fail_notice_once.clone();
            move |_, message| {
                if message.parts.as_deref().is_some_and(|parts| {
                    parts.iter().any(|part| {
                        matches!(
                            part,
                            MessagePart::SystemNotification {
                                notification_type: SystemNotificationType::SessionRecovery,
                                ..
                            }
                        )
                    })
                }) && fail_notice_once.swap(false, Ordering::SeqCst)
                {
                    return Err("injected recovery notice persistence failure".to_string());
                }
                Ok(())
            }
        }));
        let mut completion = usecase
            .ctx
            .sessions
            .lock()
            .await
            .get(&session.id)
            .and_then(|state| state.backend_recovery.as_ref())
            .expect("recovery is in progress before fresh establishment")
            .completion
            .subscribe();

        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "fresh-thread".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), completion.changed())
            .await
            .expect("recovery completion signal is sent")
            .unwrap();
        assert!(*completion.borrow());
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let recovery_finished = usecase
                    .ctx
                    .sessions
                    .lock()
                    .await
                    .get(&session.id)
                    .is_none_or(|state| state.backend_recovery.is_none());
                if recovery_finished {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        let committed = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_ne!(committed.state, SessionState::Error);
        assert_eq!(committed.agent_session_id.as_deref(), Some("fresh-thread"));
        assert!(committed.pending_recovery_message.is_some());
        workspace_query.replace_session_summaries(vec![committed.to_summary()], Vec::new());
        let listed = usecase
            .list_sessions(tmp.path().to_string_lossy().as_ref())
            .await
            .unwrap();
        assert_eq!(
            listed
                .iter()
                .find(|summary| summary.id == session.id)
                .unwrap()
                .agent_session_id
                .as_deref(),
            Some("fresh-thread"),
            "the publication snapshot is removed after the Completed commit"
        );

        usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "next turn is not blocked".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_ne!(loaded.session.state, SessionState::Error);
        assert_eq!(
            loaded
                .session
                .messages
                .iter()
                .flat_map(|message| message.parts.as_deref().unwrap_or_default())
                .filter(|part| matches!(
                    part,
                    MessagePart::SystemNotification {
                        notification_type: SystemNotificationType::SessionRecovery,
                        ..
                    }
                ))
                .count(),
            1
        );
        assert!(session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap()
            .pending_recovery_message
            .is_none());
        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryFailed { .. }
        )));
    }

    #[tokio::test]
    async fn unfinished_durable_recovery_is_reconciled_and_blocks_new_turns_after_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let original_store = Arc::new(build_session_store());
        let session = create_session_internal_with_attributes(
            &original_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        original_store
            .begin_backend_session_recovery(
                tmp.path(),
                &session.id,
                "interrupted-recovery",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        drop(original_store);

        let reopened_store = Arc::new(build_session_store());
        let (usecase, _) =
            build_agent_runtime_usecase_with_controller(reopened_store.clone(), tmp.path());
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.session.state, SessionState::Error);
        assert!(loaded.session.messages.iter().any(|message| {
            message.parts.as_deref().is_some_and(|parts| {
                parts
                    .iter()
                    .any(|part| matches!(part, MessagePart::Error { .. }))
            })
        }));
        let message_count = loaded.session.messages.len();

        let result = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "must remain blocked".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("codex".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await;
        assert!(result.is_err());
        assert_eq!(
            reopened_store
                .load_full_session_for_restore(tmp.path(), &session.id)
                .unwrap()
                .unwrap()
                .messages
                .len(),
            message_count
        );
    }

    #[tokio::test]
    async fn public_send_and_workflow_lock_wait_for_recovery_completion() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("dead-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        controller.fail_next_resume_open();
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;

        let send_usecase = Arc::clone(&usecase);
        let send_session = session.id.clone();
        let send_worktree = tmp.path().to_string_lossy().to_string();
        let send = tokio::spawn(async move {
            send_usecase
                .send_message(SendAgentMessageRequest {
                    chat_session_id: Some(send_session),
                    worktree_path: send_worktree,
                    content: "after recovery".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    backend_id: Some("codex".to_string()),
                    model_id: None,
                    images: None,
                    mentions: None,
                    editor_context: None,
                })
                .await
        });
        let workflow_usecase = Arc::clone(&usecase);
        let workflow_session = session.id.clone();
        let workflow_lock = tokio::spawn(async move {
            let guard = workflow_usecase
                .acquire_session_control_after_recovery(&workflow_session)
                .await;
            drop(guard);
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!send.is_finished());
        assert!(!workflow_lock.is_finished());

        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "fresh-thread".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        send.await.unwrap().unwrap();
        workflow_lock.await.unwrap();
        assert!(session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
            )));
    }

    #[tokio::test]
    async fn public_close_waits_for_recovery_and_closed_state_does_not_reconcile_to_error() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("dead-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        controller.fail_next_resume_open();
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;

        let close_usecase = Arc::clone(&usecase);
        let close_session_id = session.id.clone();
        let close =
            tokio::spawn(async move { close_usecase.close_session(&close_session_id).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!close.is_finished());

        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "fresh-thread".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        close.await.unwrap().unwrap();
        crate::usecase::agent_session::session::lifecycle_controller::SessionLifecycleController {
            session_store: &session_store,
            data_dir: tmp.path(),
        }
        .close_session_state(&session.id)
        .unwrap();

        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryCompleted { .. }
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryFailed { .. }
        )));
        let reopened_store = Arc::new(build_session_store());
        let (reopened_usecase, _) =
            build_agent_runtime_usecase_with_controller(reopened_store, tmp.path());
        let reopened = reopened_usecase
            .get_session(&session.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reopened.session.state, SessionState::Closed);
    }

    #[tokio::test]
    async fn force_close_does_not_wait_for_recovery_establishment() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            build_agent_runtime_usecase_with_controller(session_store.clone(), tmp.path());
        let mut session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("codex".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("gpt-5.6-sol".to_string()),
                plan_mode: false,
                workflow_node_session: false,
                workflow_node_context: None,
            },
        )
        .unwrap();
        session.agent_session_id = Some("dead-thread".to_string());
        session_store
            .save_full_session_for_restore(tmp.path(), &session)
            .unwrap();
        controller.fail_next_resume_open();
        usecase
            .start_session(
                &session.id,
                StartSessionOptions {
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        wait_for_open_count(&controller, &session.id, 2).await;
        assert!(usecase
            .ctx
            .sessions
            .lock()
            .await
            .get(&session.id)
            .is_some_and(|state| state.backend_recovery.is_some()));

        tokio::time::timeout(
            Duration::from_millis(200),
            usecase.force_close_session(&session.id),
        )
        .await
        .expect("force close must not wait for SessionEstablished")
        .unwrap();
    }

    #[tokio::test]
    async fn test_start_turn_locked_workflow_contextのstale_timeoutは_session_specへ渡さない() {
        // Given: a workflow-step session with explicit startup/stale timeout hints.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(Some(12), Some(3), Some(44))),
            },
        )
        .unwrap();

        // When: the workflow starts a turn.
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();

        // Then: startup hints are passed to the backend, but stale timeout remains
        // owned by the Rust stall watchdog and is not passed to backend stream watchdogs.
        assert!(controller.calls().iter().any(|call| {
            call.session_id == session.id
                && call.kind
                    == TestRuntimeCallKind::OpenSession {
                        startup_timeout_ms: Some(12_000),
                        startup_max_retries: Some(3),
                        stale_timeout_ms: None,
                        resume: None,
                        model: "claude-sonnet-5".to_string(),
                        permission_mode: PermissionMode::Edit,
                        plan_mode: false,
                    }
        }));
    }

    #[tokio::test]
    async fn start_turn_locked_rejects_a_durably_paused_workflow_session_until_resume() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, None)),
            },
        )
        .unwrap();
        session_store
            .append_session_event_and_project_state(
                tmp.path(),
                &session.id,
                AgentSessionEvent::QueuePaused { at: 42.0 },
            )
            .unwrap();
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );

        let error = usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "must wait for resume".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("Agent queue is paused"));
        assert_eq!(usecase.turn_phase(&session.id).await, Some(TurnPhase::Idle));
        assert!(usecase
            .get_session(&session.id)
            .await
            .unwrap()
            .unwrap()
            .session
            .messages
            .is_empty());
        let events = session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap();
        assert!(!events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::TurnStarted { .. })));
        assert!(!controller
            .call_kinds_for(&session.id)
            .iter()
            .any(|kind| matches!(kind, TestRuntimeCallKind::StartTurn)));

        usecase.resume_queue(&session.id).await.unwrap();
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run after resume".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 1).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn interrupt_before_turn_state_commit_prevents_provider_start_and_persists_pause() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, None)),
            },
        )
        .unwrap();
        let gate = Arc::new((Mutex::new((false, false)), Condvar::new()));
        session_store.set_append_event_hook_for_test({
            let gate = Arc::clone(&gate);
            Arc::new(move |_, event| {
                if matches!(event, AgentSessionEvent::TurnStarted { .. }) {
                    let (lock, condvar) = &*gate;
                    let mut state = lock.lock().unwrap();
                    state.0 = true;
                    condvar.notify_all();
                    while !state.1 {
                        state = condvar.wait(state).unwrap();
                    }
                }
                Ok(())
            })
        });
        let start = {
            let usecase = Arc::clone(&usecase);
            let session_id = session.id.clone();
            tokio::spawn(async move {
                usecase
                    .start_turn_locked(
                        &session_id,
                        PermissionMode::Edit,
                        "run".to_string(),
                        None,
                        Vec::new(),
                    )
                    .await
            })
        };
        {
            let (lock, condvar) = &*gate;
            let mut state = lock.lock().unwrap();
            while !state.0 {
                let (next, timeout) = condvar.wait_timeout(state, Duration::from_secs(1)).unwrap();
                assert!(
                    !timeout.timed_out(),
                    "TurnStarted append hook was not reached"
                );
                state = next;
            }
        }
        {
            let sessions = usecase.ctx.sessions.lock().await;
            let state = sessions
                .get(&session.id)
                .expect("turn start intent must be registered before durable append");
            assert_eq!(state.projected_turn_phase(), TurnPhase::Streaming);
            assert_eq!(state.active_turn_id(), state.last_turn_id());
            assert!(state.active_turn_id().is_some());
            assert_eq!(state.generation(), 1);
            assert!(
                state.turn_started_at().is_none(),
                "reset_for_turn state must remain uncommitted at the TurnStarted append hook"
            );
            assert!(state.last_progress_at().is_none());
        }

        usecase.interrupt(&session.id).await.unwrap();

        assert!(session_store
            .load_queue_paused_at(tmp.path(), &session.id)
            .unwrap()
            .is_some());
        assert!(session_store
            .load_session_events(tmp.path(), &session.id)
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::TurnInterruptRequested { .. })));
        assert!(!controller
            .call_kinds_for(&session.id)
            .iter()
            .any(|kind| matches!(kind, TestRuntimeCallKind::StartTurn)));
        {
            let (lock, condvar) = &*gate;
            let mut state = lock.lock().unwrap();
            state.1 = true;
            condvar.notify_all();
        }
        start
            .await
            .unwrap()
            .expect("accepted Stop owns the interrupted start");
        assert!(!controller
            .call_kinds_for(&session.id)
            .iter()
            .any(|kind| matches!(kind, TestRuntimeCallKind::StartTurn)));

        let restarted =
            crate::test_support::build_agent_runtime_usecase(session_store.clone(), tmp.path());
        assert!(
            restarted
                .get_session(&session.id)
                .await
                .unwrap()
                .unwrap()
                .queue_paused
        );
    }

    #[tokio::test]
    async fn test_stale_watchdog_無進捗turnをstall_signalに留めruntimeを閉じない() {
        // Given: a workflow-step session whose stale timeout is immediate for the test.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        usecase.set_workflow_stall_notifier(workflow_stall_notifier.clone());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, Some(0))),
            },
        )
        .unwrap();

        // When: a turn starts and no runtime progress arrives.
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_stall_observation_count(&event_notifier, 1).await;
        let images = vec![ImageAttachment {
            data: "iVBORw==".to_string(),
            media_type: "image/png".to_string(),
        }];
        let mentions = vec![crate::domain::code::MentionReference {
            file_path: "src/main.rs".to_string(),
            start_line: Some(10),
            end_line: Some(20),
        }];
        let editor_context = AgentEditorContext {
            active_editor_path: Some("src/main.rs".to_string()),
            open_editor_paths: vec!["src/main.rs".to_string(), "README.md".to_string()],
            selection: Some(AgentEditorSelection {
                file_path: "src/main.rs".to_string(),
                start_line: 10,
                end_line: 20,
            }),
        };
        let response = tokio::time::timeout(
            Duration::from_millis(200),
            usecase.send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "next".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: Some(images.clone()),
                mentions: Some(mentions.clone()),
                editor_context: Some(editor_context.clone()),
            }),
        )
        .await
        .expect("send_message must not wait for stale recovery")
        .expect("stalled active turn on a non-steering backend must queue");
        wait_for_workflow_stall_notification_count(&workflow_stall_notifier, 1).await;

        // Then: the watchdog remains non-terminal and the follow-up is durably queued.
        assert!(response.agent_message.is_none());
        assert!(response.queued_turn.is_some());
        assert_eq!(response.pending_queue_count, 1);
        {
            let sessions = usecase.ctx.sessions.lock().await;
            let queued = sessions
                .get(&session.id)
                .and_then(|state| state.accepted_input_effects.values().next())
                .expect("stalled follow-up must remain in the pending queue");
            assert_eq!(queued.content, "next");
            assert_eq!(queued.images, images);
            assert_eq!(queued.mentions, mentions);
            assert_eq!(queued.editor_context, Some(editor_context));
            assert_eq!(
                queued.existing_human_message_id.as_deref(),
                Some(response.human_message.id.as_str())
            );
        }
        let calls = controller.call_kinds_for(&session.id);
        assert!(calls.contains(&TestRuntimeCallKind::Reconnect));
        assert!(!calls.contains(&TestRuntimeCallKind::Interrupt));
        assert!(!calls.contains(&TestRuntimeCallKind::Close));
        assert_eq!(
            usecase.turn_phase(&session.id).await,
            Some(TurnPhase::Streaming)
        );
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.session.state, SessionState::Active);
        assert!(loaded
            .session
            .messages
            .iter()
            .any(|message| message.id == response.human_message.id));
        assert!(event_notifier.stall_observations().iter().any(|payload| {
            payload.chat_session_id == session.id
                && payload.turn_phase == TurnPhase::Streaming
                && payload.signal_count >= 1
        }));
        assert!(workflow_stall_notifier
            .notifications()
            .iter()
            .any(|payload| {
                payload.chat_session_id == session.id
                    && payload.turn_phase == "streaming"
                    && payload.signal_count >= 1
            }));

        // And: completion of the stalled turn drains the queued follow-up into a new turn.
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            )
            .unwrap();
        wait_for_start_prompt_count(&controller, &session.id, 2).await;
        assert!(usecase.pending_queue(&session.id).await.is_empty());
        assert!(controller.call_kinds_for(&session.id).contains(
            &TestRuntimeCallKind::StartTurnPrompt {
                prompt: "next".to_string(),
            }
        ));
    }

    #[tokio::test]
    async fn test_stall_signal後のsend_messageはactive_turnへsteerしqueueしない() {
        // Given: a stalled workflow-step turn backed by a runtime that supports active-turn steering.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        usecase.set_workflow_stall_notifier(workflow_stall_notifier.clone());
        controller.enable_steering();
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, Some(0))),
            },
        )
        .unwrap();
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_stall_observation_count(&event_notifier, 1).await;

        // When: retry/continue text is sent after the stall signal.
        let response = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "continue".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        // Then: the command reaches the active turn through steer and is not trapped behind the queue.
        assert!(response.agent_message.is_none());
        assert!(response.queued_turn.is_none());
        assert_eq!(response.pending_queue_count, 0);
        assert!(usecase.pending_queue(&session.id).await.is_empty());
        assert!(controller.call_kinds_for(&session.id).contains(
            &TestRuntimeCallKind::SteerPrompt {
                prompt: "continue".to_string(),
            }
        ));
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::StartTurnPrompt { .. }))
                .count(),
            1,
            "steered intervention must not start a second turn"
        );
    }

    #[tokio::test]
    async fn test_stall_signal後のsteer失敗はhuman_messageを保存しない() {
        // Given: a stalled workflow-step turn backed by a runtime that advertises steering.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        usecase.set_workflow_stall_notifier(workflow_stall_notifier);
        controller.enable_steering();
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, Some(0))),
            },
        )
        .unwrap();
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_stall_observation_count(&event_notifier, 1).await;
        let before = usecase.get_session(&session.id).await.unwrap().unwrap();
        let before_message_count = before.session.messages.len();
        controller.fail_next_steer();

        // When: retry/continue text fails during active-turn steering.
        let send_error = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "continue".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .expect_err("steer failure must surface to the caller");

        // Then: the failed intervention is neither durable chat history nor a queued turn.
        assert!(
            format!("{send_error:?}").contains("injected test steer failure"),
            "unexpected steer error: {send_error:?}"
        );
        assert!(controller.call_kinds_for(&session.id).contains(
            &TestRuntimeCallKind::SteerPrompt {
                prompt: "continue".to_string(),
            }
        ));
        assert!(usecase.pending_queue(&session.id).await.is_empty());
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.session.messages.len(), before_message_count);
        assert!(!loaded
            .session
            .messages
            .iter()
            .any(|message| message.content == "continue"));
    }

    #[tokio::test]
    async fn test_stall_signal後のbackend進捗はsend_messageをqueueへ戻す() {
        // Given: an active turn whose previous stall signal made intervention routing available.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        usecase.set_workflow_stall_notifier(workflow_stall_notifier.clone());
        controller.enable_steering();
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, None)),
            },
        )
        .unwrap();
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_turn_phase(&usecase, &session.id, TurnPhase::Streaming).await;
        mark_stall_observation_active_for_test(&usecase, &session.id).await;

        // When: backend output resumes after the stall observation.
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::Text {
                    content: "still running".to_string(),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        wait_for_stream_delta_count(&event_notifier, 1).await;
        wait_for_workflow_stall_cleared_count(&workflow_stall_notifier, 1).await;
        wait_for_stall_clear_count(&event_notifier, 1).await;

        // Then: the signal counter is retained for the turn cap, but delivery routing is no
        // longer considered an active stall intervention.
        {
            let sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get(&session.id).unwrap();
            assert_eq!(state.stall_signal_count_for_test(), 1);
            assert!(!state.stall_observation_is_active());
        }
        assert_eq!(
            workflow_stall_notifier
                .cleared_notifications()
                .last()
                .map(|notification| notification.chat_session_id.as_str()),
            Some(session.id.as_str())
        );
        assert_eq!(event_notifier.stall_clears().last(), Some(&session.id));
        let response = usecase
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session.id.clone()),
                worktree_path: tmp.path().to_string_lossy().to_string(),
                content: "after progress".to_string(),
                permission_mode: PermissionMode::Edit,
                plan_mode: false,
                backend_id: Some("claude".to_string()),
                model_id: None,
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();

        assert!(response.agent_message.is_none());
        assert!(response.queued_turn.is_some());
        assert_eq!(response.pending_queue_count, 1);
        assert!(!controller.call_kinds_for(&session.id).contains(
            &TestRuntimeCallKind::SteerPrompt {
                prompt: "after progress".to_string(),
            }
        ));
    }

    #[tokio::test]
    async fn test_stall_signal後のkeepaliveはworkflow_stallをclearする() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        usecase.set_workflow_stall_notifier(workflow_stall_notifier.clone());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, None)),
            },
        )
        .unwrap();
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_turn_phase(&usecase, &session.id, TurnPhase::Streaming).await;
        mark_stall_observation_active_for_test(&usecase, &session.id).await;

        controller
            .emit(&session.id, AgentRuntimeEvent::KeepAlive)
            .unwrap();
        wait_for_workflow_stall_cleared_count(&workflow_stall_notifier, 1).await;
        wait_for_stall_clear_count(&event_notifier, 1).await;

        let sessions = usecase.ctx.sessions.lock().await;
        let state = sessions.get(&session.id).unwrap();
        assert!(!state.stall_observation_is_active());
        assert_eq!(event_notifier.stall_clears().last(), Some(&session.id));
        assert_eq!(
            workflow_stall_notifier
                .cleared_notifications()
                .last()
                .map(|notification| notification.chat_session_id.as_str()),
            Some(session.id.as_str())
        );
    }

    #[tokio::test]
    async fn test_stall_signal中のbackend進捗はworkflow_observe後にclearされる() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier,
            status_notifier,
        );
        usecase.set_workflow_stall_notifier(workflow_stall_notifier.clone());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, None)),
            },
        )
        .unwrap();
        workflow_stall_notifier.set_stall_observed_hook({
            let controller = controller.clone();
            let session_id = session.id.clone();
            Arc::new(move || {
                controller
                    .emit(&session_id, AgentRuntimeEvent::KeepAlive)
                    .unwrap();
            })
        });
        workflow_stall_notifier.set_stall_observed_record_delay(Duration::from_millis(50));

        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_turn_phase(&usecase, &session.id, TurnPhase::Streaming).await;
        let generation = {
            let mut sessions = usecase.ctx.sessions.lock().await;
            let state = sessions.get_mut(&session.id).unwrap();
            state.restore_runtime_progress_for_test(
                Some(std::time::Instant::now() - Duration::from_secs(1)),
                crate::usecase::agent_session::runtime::stale::MAX_STALL_SIGNALS - 1,
                crate::usecase::agent_session::runtime::stale::MAX_STALL_RECOVERY_ATTEMPTS,
                false,
            );
            state.generation()
        };

        spawn_stale_watchdog_task(
            &usecase.ctx,
            session.id.clone(),
            generation,
            Duration::from_millis(1),
        );

        wait_for_workflow_stall_notification_count(&workflow_stall_notifier, 1).await;
        wait_for_workflow_stall_cleared_count(&workflow_stall_notifier, 1).await;

        assert_eq!(
            workflow_stall_notifier.event_order(),
            vec!["observed", "cleared"]
        );
        let sessions = usecase.ctx.sessions.lock().await;
        let state = sessions.get(&session.id).unwrap();
        assert!(!state.stall_observation_is_active());
    }

    #[tokio::test]
    async fn test_stale_watchdogはreconnect未対応backendでも介入点提示に留める() {
        // Given: a workflow-step session backed by a runtime whose reconnect capability is unavailable.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        controller.make_reconnect_unavailable();
        usecase.set_workflow_stall_notifier(workflow_stall_notifier.clone());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, Some(0))),
            },
        )
        .unwrap();

        // When: the turn reaches the stale threshold without backend progress.
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_stall_observation_count(&event_notifier, 1).await;
        wait_for_workflow_stall_notification_count(&workflow_stall_notifier, 1).await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Then: Unavailable reconnect falls back to intervention signaling only.
        assert!(event_notifier.stall_observations().iter().any(|payload| {
            payload.chat_session_id == session.id && payload.turn_phase == TurnPhase::Streaming
        }));
        assert!(workflow_stall_notifier
            .notifications()
            .iter()
            .any(|payload| payload.chat_session_id == session.id
                && payload.turn_phase == "streaming"));
        let calls = controller.call_kinds_for(&session.id);
        assert!(!calls.contains(&TestRuntimeCallKind::Reconnect));
        assert!(!calls.contains(&TestRuntimeCallKind::Interrupt));
        assert!(!calls.contains(&TestRuntimeCallKind::Close));
        assert_eq!(
            usecase.turn_phase(&session.id).await,
            Some(TurnPhase::Streaming)
        );
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.session.state, SessionState::Active);
    }

    #[tokio::test]
    async fn test_stale_watchdogはreconnect_other失敗でも非破壊で上限までretryする() {
        // Given: a workflow-step session whose reconnect attempts fail with a generic backend
        // error rather than Unavailable.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let workflow_stall_notifier = Arc::new(RecordingWorkflowStallNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        for _ in 0..crate::usecase::agent_session::runtime::stale::MAX_STALL_RECOVERY_ATTEMPTS {
            controller.fail_next_reconnect();
        }
        usecase.set_workflow_stall_notifier(workflow_stall_notifier.clone());
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, Some(0))),
            },
        )
        .unwrap();

        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_stall_observation_count(
            &event_notifier,
            crate::usecase::agent_session::runtime::stale::MAX_STALL_SIGNALS as usize,
        )
        .await;
        wait_for_workflow_stall_notification_count(&workflow_stall_notifier, 1).await;
        wait_for_call_count(
            &controller,
            &session.id,
            TestRuntimeCallKind::Reconnect,
            crate::usecase::agent_session::runtime::stale::MAX_STALL_RECOVERY_ATTEMPTS as usize,
        )
        .await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(event_notifier.stall_observations().iter().any(|payload| {
            payload.chat_session_id == session.id && payload.turn_phase == TurnPhase::Streaming
        }));
        assert!(workflow_stall_notifier
            .notifications()
            .iter()
            .any(|payload| payload.chat_session_id == session.id
                && payload.turn_phase == "streaming"));
        let calls = controller.call_kinds_for(&session.id);
        assert_eq!(
            calls
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::Reconnect))
                .count(),
            crate::usecase::agent_session::runtime::stale::MAX_STALL_RECOVERY_ATTEMPTS as usize
        );
        assert!(!calls.contains(&TestRuntimeCallKind::Interrupt));
        assert!(!calls.contains(&TestRuntimeCallKind::Close));
        assert_eq!(
            usecase.turn_phase(&session.id).await,
            Some(TurnPhase::Streaming)
        );
        let loaded = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.session.state, SessionState::Active);
    }

    #[tokio::test]
    async fn test_stale_watchdogはツール実行中のturnにstall_signalを出さない() {
        // Given: a workflow-step session with a 1-second stale timeout.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let (usecase, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                tmp.path(),
            );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, Some(1))),
            },
        )
        .unwrap();

        // When: a turn starts and a tool call is dispatched whose result has not
        // arrived (a long-running command keeps the backend silent).
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::PartsMerged(vec![DomainMessagePart::ToolUse {
                    id: "tool-1".to_string(),
                    tool: "Bash".to_string(),
                    input: crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                        "{}".to_string(),
                    ),
                    parent_tool_use_id: None,
                }]),
            )
            .unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Then: the stale watchdog does not interrupt or recover the healthy tool-in-flight turn.
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Interrupt));
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Reconnect));
    }

    #[tokio::test]
    async fn test_stall_signal後のbackend明示abort完了でturnを確定する() {
        // Given: a workflow-step session whose stale timeout is observed before the backend
        // reports its explicit interrupt result.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, Some(0))),
            },
        )
        .unwrap();
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_stall_observation_count(&event_notifier, 1).await;
        assert_eq!(
            usecase.turn_phase(&session.id).await,
            Some(TurnPhase::Streaming)
        );

        // When: the backend reports an abort completion after the stall signal.
        controller
            .emit(
                &session.id,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                    reason: DomainInterruptReason::Abort,
                    error: None,
                }),
            )
            .unwrap();
        wait_for_turn_phase(&usecase, &session.id, TurnPhase::Idle).await;

        // Then: the explicit backend terminal event, not the stall signal, determines the turn.
        let calls = controller.call_kinds_for(&session.id);
        assert!(!calls.contains(&TestRuntimeCallKind::Interrupt));
        assert!(!calls.contains(&TestRuntimeCallKind::Close));
        let session = usecase.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(session.session.state, SessionState::Idle);
    }

    #[tokio::test]
    async fn test_stale_watchdogはstall_signalとreconnectを上限で止める() {
        // Given: a workflow-step session whose stale timeout is immediate for the test.
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(build_session_store());
        let event_notifier = Arc::new(RecordingAgentNotifier::default());
        let status_notifier = Arc::new(RecordingStatusNotifier::default());
        let (usecase, controller) = build_agent_runtime_usecase_with_controller_and_notifiers(
            session_store.clone(),
            tmp.path(),
            event_notifier.clone(),
            status_notifier,
        );
        let session = create_session_internal_with_attributes(
            &session_store,
            tmp.path(),
            tmp.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            PermissionMode::Edit,
            SessionCreationAttributes {
                selected_model: Some("claude-sonnet-5".to_string()),
                plan_mode: false,
                workflow_node_session: true,
                workflow_node_context: Some(workflow_node_context(None, None, Some(0))),
            },
        )
        .unwrap();

        // When: a turn starts and remains silent past repeated stale observations.
        usecase
            .start_turn_locked(
                &session.id,
                PermissionMode::Edit,
                "run".to_string(),
                None,
                Vec::new(),
            )
            .await
            .unwrap();
        wait_for_stall_observation_count(
            &event_notifier,
            crate::usecase::agent_session::runtime::stale::MAX_STALL_SIGNALS as usize,
        )
        .await;
        wait_for_call_count(
            &controller,
            &session.id,
            TestRuntimeCallKind::Reconnect,
            crate::usecase::agent_session::runtime::stale::MAX_STALL_RECOVERY_ATTEMPTS as usize,
        )
        .await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Then: signals/reconnects are capped and the session is still live.
        let observations = event_notifier.stall_observations();
        assert_eq!(
            observations.len(),
            crate::usecase::agent_session::runtime::stale::MAX_STALL_SIGNALS as usize
        );
        assert!(observations.last().is_some_and(|payload| {
            payload.cap_reached
                && payload.signal_count
                    == crate::usecase::agent_session::runtime::stale::MAX_STALL_SIGNALS
        }));
        assert_eq!(
            controller
                .call_kinds_for(&session.id)
                .iter()
                .filter(|kind| matches!(kind, TestRuntimeCallKind::Reconnect))
                .count(),
            crate::usecase::agent_session::runtime::stale::MAX_STALL_RECOVERY_ATTEMPTS as usize
        );
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Interrupt));
        assert!(!controller
            .call_kinds_for(&session.id)
            .contains(&TestRuntimeCallKind::Close));
        assert_eq!(
            usecase.turn_phase(&session.id).await,
            Some(TurnPhase::Streaming)
        );
    }
}
