use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::{mpsc, Notify};

use crate::adaptor::gateway::agent_session::FileSessionStorage;
use crate::domain::agent_session::entities::PermissionResponse;
use crate::domain::agent_session::gateway::{
    AgentBackend, AgentBackendError, AgentRuntimeEvent, AgentSessionRuntime, ForkSessionRequest,
    SessionSpec, TurnInput,
};
use crate::domain::agent_session::value_objects::{
    BackendCapabilities, ModelDescriptor, ModelId, PermissionMode, SkillEntry,
};
use crate::usecase::agent_session::backend_registry::AgentBackendRegistry;
use crate::usecase::agent_session::context::InstructionSourcePort;
use crate::usecase::agent_session::runtime::ports::{
    AgentSessionEventNotifier, AgentSessionStateChangedPayload, AgentStallObservedPayload,
    AgentStreamingDeltaPayload, AgentTaskSpawner,
};
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
use crate::usecase::agent_session::session::SessionStore;
use crate::usecase::agent_session::status::{
    AgentStatusCenter, AgentStatusChanges, AgentStatusNotifier,
};

pub(crate) mod git;

mod agent_session_wire_replay;

pub(crate) static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

pub(crate) struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    pub(crate) fn set_value(key: &'static str, value: &str) -> Self {
        Self::set_os(key, OsStr::new(value))
    }

    pub(crate) fn set_path(key: &'static str, value: &Path) -> Self {
        Self::set_os(key, value.as_os_str())
    }

    fn set_os(key: &'static str, value: &OsStr) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

pub(crate) fn build_session_store() -> SessionStore {
    SessionStore::new(Arc::new(FileSessionStorage::default()))
}

pub(crate) fn build_agent_runtime_usecase(
    session_store: Arc<SessionStore>,
    data_dir: impl Into<std::path::PathBuf>,
) -> Arc<AgentSessionRuntimeUsecase> {
    build_agent_runtime_usecase_with_controller(session_store, data_dir).0
}

pub(crate) fn build_agent_runtime_usecase_with_controller(
    session_store: Arc<SessionStore>,
    data_dir: impl Into<std::path::PathBuf>,
) -> (Arc<AgentSessionRuntimeUsecase>, TestAgentRuntimeController) {
    build_agent_runtime_usecase_with_controller_and_notifiers(
        session_store,
        data_dir,
        Arc::new(NoopAgentSessionEventNotifier),
        Arc::new(NoopAgentStatusNotifier),
    )
}

pub(crate) fn build_agent_runtime_usecase_with_controller_and_notifiers(
    session_store: Arc<SessionStore>,
    data_dir: impl Into<std::path::PathBuf>,
    event_notifier: Arc<dyn AgentSessionEventNotifier>,
    status_notifier: Arc<dyn AgentStatusNotifier>,
) -> (Arc<AgentSessionRuntimeUsecase>, TestAgentRuntimeController) {
    let controller = TestAgentRuntimeController::default();
    let mut registry = AgentBackendRegistry::new();
    registry.register(Arc::new(TestAgentBackend {
        id: "claude",
        name: "Claude",
        models: vec!["claude-4-sonnet", "claude-opus-4-8"],
        controller: controller.clone(),
    }));
    registry.register(Arc::new(TestAgentBackend {
        id: "codex",
        name: "Codex",
        models: vec![
            "gpt-5",
            "gpt-5.6-sol",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
        ],
        controller: controller.clone(),
    }));
    let usecase = Arc::new(AgentSessionRuntimeUsecase::new(
        session_store.clone(),
        Arc::new(registry),
        Arc::new(AgentStatusCenter::new()),
        status_notifier,
        event_notifier,
        Arc::new(TokioTestAgentTaskSpawner),
        None,
        Arc::new(EmptyInstructionSource),
        data_dir.into(),
    ));
    crate::adaptor::controller::event_log_recovery_wiring::register_event_log_recovery_listener(
        session_store,
        &usecase,
    );
    (usecase, controller)
}

#[derive(Clone, Default)]
pub(crate) struct TestAgentRuntimeController {
    senders: Arc<Mutex<HashMap<String, Vec<mpsc::UnboundedSender<AgentRuntimeEvent>>>>>,
    calls: Arc<Mutex<Vec<TestRuntimeCall>>>,
    open_session_gate: Arc<Mutex<Option<Arc<Notify>>>>,
    open_session_failures: Arc<Mutex<usize>>,
    respond_permission_gate: Arc<Mutex<Option<Arc<Notify>>>>,
    start_turn_gate: Arc<Mutex<Option<Arc<Notify>>>>,
    next_start_turn_gate: Arc<Mutex<Option<Arc<Notify>>>>,
    start_turn_failures: Arc<Mutex<usize>>,
    interrupt_failures: Arc<Mutex<usize>>,
    respond_permission_failures: Arc<Mutex<usize>>,
    steer_failures: Arc<Mutex<usize>>,
    steering_available: Arc<Mutex<bool>>,
    reconnect_unavailable: Arc<Mutex<bool>>,
    reconnect_failures: Arc<Mutex<usize>>,
    resume_open_failures: Arc<Mutex<usize>>,
    open_failures: Arc<Mutex<usize>>,
}

impl TestAgentRuntimeController {
    pub(crate) fn pause_open_session(&self) {
        *self.open_session_gate.lock().unwrap() = Some(Arc::new(Notify::new()));
    }

    pub(crate) fn release_open_session(&self) {
        if let Some(gate) = self.open_session_gate.lock().unwrap().take() {
            gate.notify_waiters();
        }
    }

    fn open_session_gate(&self) -> Option<Arc<Notify>> {
        self.open_session_gate.lock().unwrap().clone()
    }

    pub(crate) fn fail_next_open_session(&self) {
        *self.open_session_failures.lock().unwrap() += 1;
    }

    fn should_fail_open_session(&self) -> bool {
        let mut failures = self.open_session_failures.lock().unwrap();
        if *failures == 0 {
            return false;
        }
        *failures -= 1;
        true
    }

    fn register(&self, session_id: String, sender: mpsc::UnboundedSender<AgentRuntimeEvent>) {
        self.senders
            .lock()
            .unwrap()
            .entry(session_id)
            .or_default()
            .push(sender);
    }

    #[allow(dead_code)] // issues-1301 G-3: event injection is retained for runtime scenario tests beyond the focused usecase suite.
    pub(crate) fn emit(&self, session_id: &str, event: AgentRuntimeEvent) -> Result<(), String> {
        let sender = self
            .senders
            .lock()
            .unwrap()
            .get(session_id)
            .and_then(|senders| senders.last())
            .cloned()
            .ok_or_else(|| format!("test runtime is not registered for session {session_id}"))?;
        sender
            .send(event)
            .map_err(|_| format!("test runtime event stream is closed for session {session_id}"))
    }

    pub(crate) fn emit_for_runtime(
        &self,
        session_id: &str,
        runtime_index: usize,
        event: AgentRuntimeEvent,
    ) -> Result<(), String> {
        let sender = self
            .senders
            .lock()
            .unwrap()
            .get(session_id)
            .and_then(|senders| senders.get(runtime_index))
            .cloned()
            .ok_or_else(|| {
                format!("test runtime {runtime_index} is not registered for session {session_id}")
            })?;
        sender
            .send(event)
            .map_err(|_| format!("test runtime event stream is closed for session {session_id}"))
    }

    #[allow(dead_code)] // issues-1301 G-3: registration inspection is retained for runtime scenario tests.
    pub(crate) fn registered_session_ids(&self) -> Vec<String> {
        self.senders.lock().unwrap().keys().cloned().collect()
    }

    fn record(&self, session_id: impl Into<String>, kind: TestRuntimeCallKind) {
        self.calls.lock().unwrap().push(TestRuntimeCall {
            session_id: session_id.into(),
            kind,
        });
    }

    pub(crate) fn calls(&self) -> Vec<TestRuntimeCall> {
        self.calls.lock().unwrap().clone()
    }

    pub(crate) fn call_kinds_for(&self, session_id: &str) -> Vec<TestRuntimeCallKind> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.session_id == session_id)
            .map(|call| call.kind.clone())
            .collect()
    }

    pub(crate) fn pause_start_turn(&self) {
        *self.start_turn_gate.lock().unwrap() = Some(Arc::new(Notify::new()));
    }

    pub(crate) fn pause_next_start_turn(&self) -> Arc<Notify> {
        let gate = Arc::new(Notify::new());
        *self.next_start_turn_gate.lock().unwrap() = Some(Arc::clone(&gate));
        gate
    }

    pub(crate) fn release_start_turn(&self) {
        if let Some(gate) = self.start_turn_gate.lock().unwrap().take() {
            gate.notify_waiters();
        }
    }

    pub(crate) fn fail_next_start_turn(&self) {
        *self.start_turn_failures.lock().unwrap() += 1;
    }

    fn should_fail_start_turn(&self) -> bool {
        let mut failures = self.start_turn_failures.lock().unwrap();
        if *failures == 0 {
            return false;
        }
        *failures -= 1;
        true
    }

    pub(crate) fn fail_next_interrupt(&self) {
        *self.interrupt_failures.lock().unwrap() += 1;
    }

    fn should_fail_interrupt(&self) -> bool {
        let mut failures = self.interrupt_failures.lock().unwrap();
        if *failures == 0 {
            return false;
        }
        *failures -= 1;
        true
    }

    fn start_turn_gate(&self) -> Option<Arc<Notify>> {
        self.next_start_turn_gate
            .lock()
            .unwrap()
            .take()
            .or_else(|| self.start_turn_gate.lock().unwrap().clone())
    }

    pub(crate) fn fail_next_respond_permission(&self) {
        *self.respond_permission_failures.lock().unwrap() += 1;
    }

    pub(crate) fn pause_respond_permission(&self) {
        *self.respond_permission_gate.lock().unwrap() = Some(Arc::new(Notify::new()));
    }

    pub(crate) fn release_respond_permission(&self) {
        if let Some(gate) = self.respond_permission_gate.lock().unwrap().take() {
            gate.notify_waiters();
        }
    }

    fn respond_permission_gate(&self) -> Option<Arc<Notify>> {
        self.respond_permission_gate.lock().unwrap().clone()
    }

    fn should_fail_respond_permission(&self) -> bool {
        let mut failures = self.respond_permission_failures.lock().unwrap();
        if *failures == 0 {
            return false;
        }
        *failures -= 1;
        true
    }

    pub(crate) fn fail_next_steer(&self) {
        *self.steer_failures.lock().unwrap() += 1;
    }

    fn should_fail_steer(&self) -> bool {
        let mut failures = self.steer_failures.lock().unwrap();
        if *failures == 0 {
            return false;
        }
        *failures -= 1;
        true
    }

    pub(crate) fn make_reconnect_unavailable(&self) {
        *self.reconnect_unavailable.lock().unwrap() = true;
    }

    fn reconnect_is_unavailable(&self) -> bool {
        *self.reconnect_unavailable.lock().unwrap()
    }

    pub(crate) fn fail_next_reconnect(&self) {
        *self.reconnect_failures.lock().unwrap() += 1;
    }

    pub(crate) fn fail_next_resume_open(&self) {
        *self.resume_open_failures.lock().unwrap() += 1;
    }

    pub(crate) fn fail_next_open(&self) {
        *self.open_failures.lock().unwrap() += 1;
    }

    fn should_fail_resume_open(&self) -> bool {
        let mut failures = self.resume_open_failures.lock().unwrap();
        if *failures == 0 {
            return false;
        }
        *failures -= 1;
        true
    }

    fn should_fail_open(&self) -> bool {
        let mut failures = self.open_failures.lock().unwrap();
        if *failures == 0 {
            return false;
        }
        *failures -= 1;
        true
    }

    fn should_fail_reconnect(&self) -> bool {
        let mut failures = self.reconnect_failures.lock().unwrap();
        if *failures == 0 {
            return false;
        }
        *failures -= 1;
        true
    }

    pub(crate) fn enable_steering(&self) {
        *self.steering_available.lock().unwrap() = true;
    }

    fn steering_is_available(&self) -> bool {
        *self.steering_available.lock().unwrap()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TestRuntimeCall {
    pub session_id: String,
    pub kind: TestRuntimeCallKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TestRuntimeCallKind {
    OpenSession {
        startup_timeout_ms: Option<u128>,
        startup_max_retries: Option<u32>,
        stale_timeout_ms: Option<u128>,
        resume: Option<String>,
        model: String,
        permission_mode: PermissionMode,
        plan_mode: bool,
    },
    StartTurn,
    StartTurnPrompt {
        prompt: String,
    },
    StartTurnEditorContext {
        editor_context: Option<crate::domain::agent_session::value_objects::EditorContext>,
    },
    StartTurnImages {
        images: Vec<crate::domain::agent_session::entities::AttachmentPayload>,
    },
    StartTurnSystemPrompt {
        system_prompt: Option<String>,
    },
    SteerPrompt {
        prompt: String,
    },
    Reconnect,
    Interrupt,
    RespondPermission {
        request_id: String,
    },
    SetPermissionMode,
    SetModel,
    Close,
}

struct TestAgentBackend {
    id: &'static str,
    name: &'static str,
    models: Vec<&'static str>,
    controller: TestAgentRuntimeController,
}

#[async_trait]
impl AgentBackend for TestAgentBackend {
    fn id(&self) -> &str {
        self.id
    }

    fn name(&self) -> &str {
        self.name
    }

    fn available_models(&self) -> Vec<ModelDescriptor> {
        self.models
            .iter()
            .map(|model| ModelDescriptor {
                id: ModelId::parse(*model).unwrap(),
                display_name: (*model).to_string(),
            })
            .collect()
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            steering: self.controller.steering_is_available(),
        }
    }

    async fn open_session(
        &self,
        spec: SessionSpec,
    ) -> Result<Box<dyn AgentSessionRuntime>, AgentBackendError> {
        let requested_resume_id = spec.resume.clone();
        let (sender, receiver) = mpsc::unbounded_channel();
        self.controller.record(
            spec.session_id.clone(),
            TestRuntimeCallKind::OpenSession {
                startup_timeout_ms: spec.startup_timeout.map(|value| value.as_millis()),
                startup_max_retries: spec.startup_max_retries,
                stale_timeout_ms: spec.stale_timeout.map(|value| value.as_millis()),
                resume: requested_resume_id.clone(),
                model: spec.model.as_str().to_string(),
                permission_mode: spec.permission_mode,
                plan_mode: spec.plan_mode,
            },
        );
        if let Some(gate) = self.controller.open_session_gate() {
            gate.notified().await;
        }
        if self.controller.should_fail_open_session() {
            return Err(AgentBackendError::Other(
                "injected test open session failure".to_string(),
            ));
        }
        if let Some(requested_resume_id) = requested_resume_id {
            if self.controller.should_fail_resume_open() {
                return Err(AgentBackendError::BackendSessionLost {
                    requested_resume_id,
                });
            }
        }
        if self.controller.should_fail_open() {
            return Err(AgentBackendError::Other(
                "injected test open failure".to_string(),
            ));
        }
        self.controller.register(spec.session_id.clone(), sender);
        Ok(Box::new(TestAgentRuntime {
            session_id: spec.session_id,
            controller: self.controller.clone(),
            receiver: Some(receiver),
        }))
    }

    async fn archive_session(
        &self,
        _backend_session_id: &str,
        _cwd: &str,
    ) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn unarchive_session(
        &self,
        _backend_session_id: &str,
        _cwd: &str,
    ) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn fork_session(
        &self,
        _req: ForkSessionRequest,
    ) -> Result<Option<String>, AgentBackendError> {
        Ok(None)
    }

    async fn skill_catalog(
        &self,
        _cwd: &std::path::Path,
        _query: Option<&str>,
        _limit: Option<usize>,
    ) -> Result<Vec<SkillEntry>, AgentBackendError> {
        Ok(Vec::new())
    }

    async fn fuzzy_file_search(
        &self,
        _root: &std::path::Path,
        _query: &str,
        _limit: usize,
    ) -> Result<Option<Vec<String>>, AgentBackendError> {
        Ok(None)
    }
}

struct TestAgentRuntime {
    session_id: String,
    controller: TestAgentRuntimeController,
    receiver: Option<mpsc::UnboundedReceiver<AgentRuntimeEvent>>,
}

#[async_trait]
impl AgentSessionRuntime for TestAgentRuntime {
    fn take_events(
        &mut self,
    ) -> std::pin::Pin<Box<dyn futures_util::Stream<Item = AgentRuntimeEvent> + Send>> {
        let Some(receiver) = self.receiver.take() else {
            return Box::pin(futures_util::stream::empty());
        };
        Box::pin(futures_util::stream::unfold(
            receiver,
            |mut receiver| async move { receiver.recv().await.map(|event| (event, receiver)) },
        ))
    }

    async fn start_turn(&self, input: TurnInput) -> Result<(), AgentBackendError> {
        self.controller
            .record(self.session_id.clone(), TestRuntimeCallKind::StartTurn);
        let gate = self.controller.start_turn_gate();
        self.controller.record(
            self.session_id.clone(),
            TestRuntimeCallKind::StartTurnPrompt {
                prompt: input.prompt.clone(),
            },
        );
        self.controller.record(
            self.session_id.clone(),
            TestRuntimeCallKind::StartTurnImages {
                images: input.images.clone(),
            },
        );
        self.controller.record(
            self.session_id.clone(),
            TestRuntimeCallKind::StartTurnSystemPrompt {
                system_prompt: input.system_prompt.clone(),
            },
        );
        self.controller.record(
            self.session_id.clone(),
            TestRuntimeCallKind::StartTurnEditorContext {
                editor_context: input.editor_context,
            },
        );
        if let Some(gate) = gate {
            gate.notified().await;
        }
        if self.controller.should_fail_start_turn() {
            return Err(AgentBackendError::Other(
                "injected test start failure".to_string(),
            ));
        }
        Ok(())
    }

    async fn interrupt(&self) -> Result<(), AgentBackendError> {
        self.controller
            .record(self.session_id.clone(), TestRuntimeCallKind::Interrupt);
        if self.controller.should_fail_interrupt() {
            return Err(AgentBackendError::Other(
                "injected test interrupt failure".to_string(),
            ));
        }
        Ok(())
    }

    async fn steer(&self, input: TurnInput) -> Result<(), AgentBackendError> {
        if !self.controller.steering_is_available() {
            return Err(AgentBackendError::Unavailable(
                "injected test steering unavailable".to_string(),
            ));
        }
        self.controller.record(
            self.session_id.clone(),
            TestRuntimeCallKind::SteerPrompt {
                prompt: input.prompt,
            },
        );
        if self.controller.should_fail_steer() {
            return Err(AgentBackendError::Other(
                "injected test steer failure".to_string(),
            ));
        }
        Ok(())
    }

    async fn reconnect(&self) -> Result<(), AgentBackendError> {
        if self.controller.reconnect_is_unavailable() {
            return Err(AgentBackendError::Unavailable(
                "injected test reconnect unavailable".to_string(),
            ));
        }
        self.controller
            .record(self.session_id.clone(), TestRuntimeCallKind::Reconnect);
        if self.controller.should_fail_reconnect() {
            return Err(AgentBackendError::Other(
                "injected test reconnect failure".to_string(),
            ));
        }
        Ok(())
    }

    async fn respond_permission(
        &self,
        response: PermissionResponse,
    ) -> Result<(), AgentBackendError> {
        self.controller.record(
            self.session_id.clone(),
            TestRuntimeCallKind::RespondPermission {
                request_id: response.request_id,
            },
        );
        if let Some(gate) = self.controller.respond_permission_gate() {
            gate.notified().await;
        }
        if self.controller.should_fail_respond_permission() {
            return Err(AgentBackendError::Other(
                "injected test permission response failure".to_string(),
            ));
        }
        Ok(())
    }

    async fn set_permission_mode(
        &self,
        _mode: crate::domain::agent_session::PermissionMode,
        _plan_mode: bool,
    ) -> Result<(), AgentBackendError> {
        self.controller.record(
            self.session_id.clone(),
            TestRuntimeCallKind::SetPermissionMode,
        );
        Ok(())
    }

    async fn set_model(&self, _model: &ModelId) -> Result<(), AgentBackendError> {
        self.controller
            .record(self.session_id.clone(), TestRuntimeCallKind::SetModel);
        Ok(())
    }

    async fn set_session_title(&self, _title: &str) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn close(&self) {
        self.controller
            .record(self.session_id.clone(), TestRuntimeCallKind::Close);
    }
}

struct TokioTestAgentTaskSpawner;

impl AgentTaskSpawner for TokioTestAgentTaskSpawner {
    fn spawn(
        &self,
        future: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>,
    ) {
        tokio::spawn(future);
    }
}

struct NoopAgentSessionEventNotifier;

impl AgentSessionEventNotifier for NoopAgentSessionEventNotifier {
    fn persist_notice(&self, _notice: crate::usecase::agent_session::status::SessionNotice) {}

    fn session_state_changed(&self, _payload: AgentSessionStateChangedPayload) {}

    fn stall_observed(&self, _payload: AgentStallObservedPayload) {}

    fn stall_cleared(&self, _session_id: &str) {}

    fn streaming_delta(&self, _payload: AgentStreamingDeltaPayload) -> bool {
        true
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

    fn permission_mode_changed(&self, _session_id: &str, _permission_mode: &str) {}

    fn models_updated(
        &self,
        _session_id: &str,
        _available_models: Vec<crate::usecase::agent_session::session::ModelInfo>,
        _selected_model: String,
    ) {
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
        _human_message: Option<crate::usecase::agent_session::session::ChatMessage>,
        _agent_message: crate::usecase::agent_session::session::ChatMessage,
    ) {
    }

    fn turn_prepared(
        &self,
        _session: &crate::usecase::agent_session::session::ChatSession,
        _human_message: &crate::usecase::agent_session::session::ChatMessage,
        _agent_message: &crate::usecase::agent_session::session::ChatMessage,
    ) {
    }
}

struct NoopAgentStatusNotifier;

impl AgentStatusNotifier for NoopAgentStatusNotifier {
    fn status_changed(&self, _changes: AgentStatusChanges) {}
}

struct EmptyInstructionSource;

impl InstructionSourcePort for EmptyInstructionSource {
    fn read_instruction_file(
        &self,
        _path: &std::path::Path,
        _worktree_root: &std::path::Path,
    ) -> Result<Option<String>, String> {
        Ok(None)
    }
}
