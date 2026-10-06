use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tempfile::{tempdir, TempDir};

use super::{
    LocalProviderLifecycleCredentialGateway, LocalProviderLifecycleEventRepository,
    ProviderLaunchContext, ProviderLaunchSpec,
};
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::domain::agent_session::{ProviderLaunchOptions, ProviderSessionLaunch};
use crate::domain::local_event::{
    CommitBatchError, CommitBatchResult, CommitIdentity, CommitResolution, DomainEventPage,
    LoadStreamRequest, LocalAtomicBatch, LocalDomainEvent, LocalEventQuery, LocalEventQueryError,
    LocalEventQueryResult, LocalEventTransactionRepository, LocalStateMutation, StreamId,
    UncommittedDomainEvent,
};
use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleIngressResult, ProviderLifecycleRejection,
    ProviderLifecycleScope, ProviderLifecycleSignal, ProviderLifecycleSlotId,
};
use crate::domain::workflow::SessionPermission;
use crate::usecase::provider_lifecycle::ProviderLifecycleUsecase;

fn scope() -> ProviderLifecycleScope {
    ProviderLifecycleScope::new("agent-1").unwrap()
}

fn context() -> ProviderLaunchContext {
    ProviderLaunchContext::new(slot_id(), "binding-1", "capability-1", scope()).unwrap()
}

fn slot_id() -> ProviderLifecycleSlotId {
    ProviderLifecycleSlotId::new("slot-1").unwrap()
}

#[test]
pub fn test_provider起動設定_development_profileでは両providerにreleash_devを使う() {
    let plugin_directory = tempdir().unwrap();
    let claude = ProviderLaunchSpec::for_provider(
        ProviderKind::Claude,
        context(),
        "releash-dev",
        Some(plugin_directory.path()),
    )
    .unwrap();
    let codex =
        ProviderLaunchSpec::for_provider(ProviderKind::Codex, context(), "releash-dev", None)
            .unwrap();

    let claude_hooks = claude
        .files()
        .iter()
        .find(|file| file.relative_path() == std::path::Path::new("hooks/hooks.json"))
        .unwrap();
    assert!(std::str::from_utf8(claude_hooks.contents())
        .unwrap()
        .contains("releash-dev hook receive --provider claude"));
    assert!(codex
        .arguments()
        .iter()
        .any(|argument| { argument.contains("releash-dev hook receive --provider codex") }));
}

#[test]
pub fn test_provider起動設定_claudeはsession_pluginを使いuser_settingsを変更しない() {
    let directory = tempdir().unwrap();
    let settings_path = directory.path().join("settings.json");
    let original = br#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"user-hook"}]}]}}"#;
    fs::write(&settings_path, original).unwrap();
    let plugin_directory = directory.path().join("launch-plugin");

    let spec = ProviderLaunchSpec::for_provider(
        ProviderKind::Claude,
        context(),
        "releash",
        Some(&plugin_directory),
    )
    .unwrap();

    assert_eq!(
        spec.arguments(),
        &[
            "--plugin-dir".to_string(),
            plugin_directory.to_string_lossy().into_owned(),
        ]
    );
    assert!(!spec.arguments().iter().any(|value| value == "--settings"));
    assert_eq!(fs::read(&settings_path).unwrap(), original);
    assert!(!spec.requires_hook_trust());

    let manifest = spec
        .files()
        .iter()
        .find(|file| file.relative_path() == std::path::Path::new(".claude-plugin/plugin.json"))
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(manifest.contents()).unwrap(),
        serde_json::json!({
            "name":"releash-provider-lifecycle",
            "version":"1.0.0",
            "description":"Releash Provider lifecycle integration",
            "author":{"name":"Releash"}
        })
    );
    let hooks = spec
        .files()
        .iter()
        .find(|file| file.relative_path() == std::path::Path::new("hooks/hooks.json"))
        .unwrap();
    let hooks = serde_json::from_slice::<serde_json::Value>(hooks.contents()).unwrap();
    for event in [
        "SessionStart",
        "Stop",
        "StopFailure",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PermissionRequest",
    ] {
        assert_eq!(
            hooks["hooks"][event][0]["hooks"][0]["command"],
            "releash hook receive --provider claude"
        );
    }
}

#[test]
pub fn test_provider起動設定_claudeのnewとresumeをstructured_root_processへ変換する() {
    let directory = tempdir().unwrap();
    let spec = ProviderLaunchSpec::for_provider(
        ProviderKind::Claude,
        context(),
        "releash",
        Some(directory.path()),
    )
    .unwrap();

    let new = spec
        .terminal_process("/opt/bin/claude", ProviderSessionLaunch::New)
        .unwrap();
    assert_eq!(new.executable(), "/opt/bin/claude");
    assert_eq!(
        new.arguments(),
        &[
            "--plugin-dir".to_string(),
            directory.path().to_string_lossy().into_owned(),
        ]
    );
    assert_eq!(new.environment(), spec.environment());

    let resumed = spec
        .terminal_process(
            "/opt/bin/claude",
            ProviderSessionLaunch::resume("claude-session-1").unwrap(),
        )
        .unwrap();
    assert_eq!(
        resumed.arguments(),
        &[
            "--plugin-dir".to_string(),
            directory.path().to_string_lossy().into_owned(),
            "--resume".to_string(),
            "claude-session-1".to_string(),
        ]
    );
}

#[test]
pub fn test_provider起動設定_claudeの初回指示を起動時promptとして渡す() {
    let directory = tempdir().unwrap();
    let spec = ProviderLaunchSpec::for_provider(
        ProviderKind::Claude,
        context(),
        "releash",
        Some(directory.path()),
    )
    .unwrap();

    let process = spec
        .terminal_process(
            "/opt/bin/claude",
            ProviderSessionLaunch::new_with_initial_instruction("Implement the workflow node.")
                .unwrap(),
        )
        .unwrap();

    assert_eq!(
        process.arguments().last().map(String::as_str),
        Some("Implement the workflow node.")
    );
}

#[test]
pub fn test_provider起動設定_permissionの4値をprovider別引数列へ写像する() {
    let cases: [(ProviderKind, SessionPermission, &[&str]); 8] = [
        (
            ProviderKind::Claude,
            SessionPermission::Manual,
            &["--permission-mode", "default"],
        ),
        (
            ProviderKind::Claude,
            SessionPermission::Auto,
            &["--permission-mode", "auto"],
        ),
        (
            ProviderKind::Claude,
            SessionPermission::Bypass,
            &["--permission-mode", "bypassPermissions"],
        ),
        (
            ProviderKind::Claude,
            SessionPermission::ReadOnly,
            &["--permission-mode", "plan"],
        ),
        (
            ProviderKind::Codex,
            SessionPermission::Manual,
            &[
                "--sandbox",
                "workspace-write",
                "--ask-for-approval",
                "on-request",
            ],
        ),
        (
            ProviderKind::Codex,
            SessionPermission::Auto,
            &["--approve-for-me"],
        ),
        (
            ProviderKind::Codex,
            SessionPermission::Bypass,
            &["--dangerously-bypass-approvals-and-sandbox"],
        ),
        (
            ProviderKind::Codex,
            SessionPermission::ReadOnly,
            &["--sandbox", "read-only", "--ask-for-approval", "never"],
        ),
    ];

    for (provider, permission, expected_permission_arguments) in cases {
        let plugin_directory = tempdir().unwrap();
        let spec = ProviderLaunchSpec::for_provider(
            provider,
            context(),
            "releash",
            (provider == ProviderKind::Claude).then_some(plugin_directory.path()),
        )
        .unwrap();
        let launch = ProviderSessionLaunch::new_with_initial_instruction("complete-action")
            .unwrap()
            .with_options(ProviderLaunchOptions::new(
                Some("model-x".to_string()),
                Some(permission),
            ));

        let process = spec.terminal_process("provider", launch).unwrap();
        let expected_suffix = ["--model", "model-x"]
            .into_iter()
            .chain(expected_permission_arguments.iter().copied())
            .chain(["complete-action"])
            .map(str::to_string)
            .collect::<Vec<_>>();

        assert!(
            process.arguments().ends_with(&expected_suffix),
            "{provider:?} {permission}: {:?}",
            process.arguments()
        );
    }
}

#[test]
pub fn test_provider起動設定_permission省略時は権限引数なしでmodelを無変換に保つ() {
    for provider in [ProviderKind::Claude, ProviderKind::Codex] {
        let plugin_directory = tempdir().unwrap();
        let spec = ProviderLaunchSpec::for_provider(
            provider,
            context(),
            "releash",
            (provider == ProviderKind::Claude).then_some(plugin_directory.path()),
        )
        .unwrap();
        let launch = ProviderSessionLaunch::new_with_initial_instruction("complete-action")
            .unwrap()
            .with_options(ProviderLaunchOptions::new(
                Some("provider-model-value".to_string()),
                None,
            ));

        let process = spec.terminal_process("provider", launch).unwrap();

        assert!(process.arguments().ends_with(&[
            "--model".to_string(),
            "provider-model-value".to_string(),
            "complete-action".to_string(),
        ]));
        assert!(process.arguments().iter().all(|argument| !matches!(
            argument.as_str(),
            "--permission-mode"
                | "--sandbox"
                | "--ask-for-approval"
                | "--approve-for-me"
                | "--dangerously-bypass-approvals-and-sandbox"
        )));
    }
}

#[test]
pub fn test_provider起動設定_launch_contextをchild_environmentだけに保持する() {
    for provider in [ProviderKind::Claude, ProviderKind::Codex] {
        let plugin_directory = tempdir().unwrap();
        let spec = ProviderLaunchSpec::for_provider(
            provider,
            context(),
            "releash",
            (provider == ProviderKind::Claude).then_some(plugin_directory.path()),
        )
        .unwrap();
        assert_eq!(
            spec.environment(),
            &[
                (
                    "RELEASH_PROVIDER_LIFECYCLE_SLOT_ID".to_string(),
                    "slot-1".to_string(),
                ),
                (
                    "RELEASH_PROVIDER_LIFECYCLE_BINDING_ID".to_string(),
                    "binding-1".to_string(),
                ),
                (
                    "RELEASH_PROVIDER_LIFECYCLE_CAPABILITY".to_string(),
                    "capability-1".to_string(),
                ),
                (
                    "RELEASH_PROVIDER_LIFECYCLE_AGENT_SESSION_ID".to_string(),
                    "agent-1".to_string(),
                ),
            ]
        );

        let visible_configuration = spec
            .arguments()
            .iter()
            .map(String::as_str)
            .chain(
                spec.files()
                    .iter()
                    .map(|file| std::str::from_utf8(file.contents()).unwrap()),
            )
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!visible_configuration.contains("binding-1"));
        assert!(!visible_configuration.contains("slot-1"));
        assert!(!visible_configuration.contains("capability-1"));
        assert!(!visible_configuration.contains("agent-1"));
        assert!(!visible_configuration.contains("workflow-1"));
        assert!(!visible_configuration.contains("node-1"));
    }
}

fn persistence_scope() -> ProviderLifecycleScope {
    ProviderLifecycleScope::new("agent-1").unwrap()
}

fn setup_persistence_usecase() -> (TempDir, Arc<LocalEventStore>, ProviderLifecycleUsecase) {
    let directory = TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
    ))
    .unwrap();
    let events = LocalProviderLifecycleEventRepository::new(
        crate::usecase::retry::shared().clone(),
        store.clone() as Arc<dyn LocalEventTransactionRepository>,
        store.installation_id().to_string(),
    );
    let usecase = ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(events),
    );
    (directory, store, usecase)
}

async fn provider_event_count(store: &LocalEventStore, agent_session_id: &str) -> usize {
    store
        .load_stream(LoadStreamRequest {
            stream_id: StreamId::provider_lifecycle(agent_session_id).unwrap(),
            after: None,
            limit: 64,
        })
        .await
        .unwrap()
        .events
        .into_iter()
        .filter(|event| {
            matches!(
                &event.event,
                crate::domain::local_event::LoadedDomainEvent::Known(inner)
                    if matches!(inner.as_ref(), LocalDomainEvent::ProviderLifecycle(_))
            )
        })
        .count()
}

struct ResolveFailureOnceRepository {
    inner: Arc<LocalEventStore>,
    fail_resolve_once: AtomicBool,
}

struct ResolveFailureRepository {
    inner: Arc<LocalEventStore>,
    fail_resolve: AtomicBool,
}

impl ResolveFailureRepository {
    fn new(inner: Arc<LocalEventStore>) -> Self {
        Self {
            inner,
            fail_resolve: AtomicBool::new(false),
        }
    }

    fn set_resolve_failure(&self, fail: bool) {
        self.fail_resolve.store(fail, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl LocalEventTransactionRepository for ResolveFailureRepository {
    fn canonical_mutation_identity_v1(
        &self,
        mutation: &LocalStateMutation,
    ) -> Result<Vec<u8>, String> {
        self.inner.canonical_mutation_identity_v1(mutation)
    }

    fn canonical_event_batch_identity_v1(
        &self,
        events: &[UncommittedDomainEvent],
    ) -> Result<Vec<u8>, String> {
        self.inner.canonical_event_batch_identity_v1(events)
    }

    async fn commit_batch(
        &self,
        batch: LocalAtomicBatch,
    ) -> Result<CommitBatchResult, CommitBatchError> {
        self.inner.commit_batch(batch).await
    }

    async fn resolve_commit(
        &self,
        identity: CommitIdentity,
    ) -> Result<CommitResolution, LocalEventQueryError> {
        if self.fail_resolve.load(Ordering::SeqCst) {
            return Err(LocalEventQueryError::QueryBusy);
        }
        self.inner.resolve_commit(identity).await
    }

    async fn load_stream(
        &self,
        request: LoadStreamRequest,
    ) -> Result<DomainEventPage, LocalEventQueryError> {
        self.inner.load_stream(request).await
    }

    async fn query(
        &self,
        request: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError> {
        self.inner.query(request).await
    }
}

impl ResolveFailureOnceRepository {
    fn new(inner: Arc<LocalEventStore>) -> Self {
        Self {
            inner,
            fail_resolve_once: AtomicBool::new(true),
        }
    }
}

#[async_trait::async_trait]
impl LocalEventTransactionRepository for ResolveFailureOnceRepository {
    fn canonical_mutation_identity_v1(
        &self,
        mutation: &LocalStateMutation,
    ) -> Result<Vec<u8>, String> {
        self.inner.canonical_mutation_identity_v1(mutation)
    }

    fn canonical_event_batch_identity_v1(
        &self,
        events: &[UncommittedDomainEvent],
    ) -> Result<Vec<u8>, String> {
        self.inner.canonical_event_batch_identity_v1(events)
    }

    async fn commit_batch(
        &self,
        batch: LocalAtomicBatch,
    ) -> Result<CommitBatchResult, CommitBatchError> {
        self.inner.commit_batch(batch).await
    }

    async fn resolve_commit(
        &self,
        identity: CommitIdentity,
    ) -> Result<CommitResolution, LocalEventQueryError> {
        if self.fail_resolve_once.swap(false, Ordering::SeqCst) {
            return Err(LocalEventQueryError::QueryBusy);
        }
        self.inner.resolve_commit(identity).await
    }

    async fn load_stream(
        &self,
        request: LoadStreamRequest,
    ) -> Result<DomainEventPage, LocalEventQueryError> {
        self.inner.load_stream(request).await
    }

    async fn query(
        &self,
        request: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError> {
        self.inner.query(request).await
    }
}

#[tokio::test]
pub async fn test_providerライフサイクル永続化_session_start再送とstopをdurableに保存する() {
    let (_directory, store, usecase) = setup_persistence_usecase();
    let armed = usecase
        .arm(slot_id(), ProviderKind::Claude, persistence_scope())
        .await
        .unwrap();
    assert_eq!(armed.provider(), ProviderKind::Claude);
    assert_eq!(armed.scope(), &persistence_scope());
    assert_eq!(provider_event_count(&store, "agent-1").await, 1);

    let started = ProviderLifecycleSignal::session_started(
        armed.binding_id(),
        ProviderKind::Claude,
        persistence_scope(),
        "claude-session-1",
        Some("provider://claude/transcript"),
    )
    .unwrap();
    assert_eq!(
        usecase
            .receive(armed.slot_id(), armed.capability(), started)
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Applied
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 2);

    let duplicate = ProviderLifecycleSignal::session_started(
        armed.binding_id(),
        ProviderKind::Claude,
        persistence_scope(),
        "claude-session-1",
        Some("provider://claude/transcript"),
    )
    .unwrap();
    assert_eq!(
        usecase
            .receive(armed.slot_id(), armed.capability(), duplicate)
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Duplicate
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 2);

    let stopped = ProviderLifecycleSignal::stop_observed(
        armed.binding_id(),
        ProviderKind::Claude,
        persistence_scope(),
        "claude-session-1",
        Some("provider://claude/transcript"),
    )
    .unwrap();
    assert_eq!(
        usecase
            .receive(armed.slot_id(), armed.capability(), stopped)
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Applied
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 3);
}

#[tokio::test]
pub async fn test_providerライフサイクル永続化_invalid_capabilityとdomain拒否ではledgerを変更しない(
) {
    let (_directory, store, usecase) = setup_persistence_usecase();
    let armed = usecase
        .arm(slot_id(), ProviderKind::Codex, persistence_scope())
        .await
        .unwrap();

    let started = ProviderLifecycleSignal::session_started(
        armed.binding_id(),
        ProviderKind::Codex,
        persistence_scope(),
        "codex-session-1",
        None,
    )
    .unwrap();
    assert_eq!(
        usecase
            .receive(armed.slot_id(), "wrong-capability", started)
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Rejected(ProviderLifecycleRejection::InvalidCapability)
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 1);

    let stale = ProviderLifecycleSignal::session_started(
        "other-binding",
        ProviderKind::Codex,
        persistence_scope(),
        "codex-session-1",
        None,
    )
    .unwrap();
    assert_eq!(
        usecase
            .receive(armed.slot_id(), armed.capability(), stale)
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Rejected(ProviderLifecycleRejection::BindingExpired)
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 1);

    let stop_before_start = ProviderLifecycleSignal::stop_observed(
        armed.binding_id(),
        ProviderKind::Codex,
        persistence_scope(),
        "codex-session-1",
        None,
    )
    .unwrap();
    assert_eq!(
        usecase
            .receive(armed.slot_id(), armed.capability(), stop_before_start)
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Rejected(ProviderLifecycleRejection::SessionNotAssociated)
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 1);
}

#[tokio::test]
pub async fn test_providerライフサイクル再設定_同一slotの旧bindingをdurableに失効させる() {
    let (_directory, store, usecase) = setup_persistence_usecase();
    let previous_scope = ProviderLifecycleScope::new("agent-previous").unwrap();
    let current_scope = ProviderLifecycleScope::new("agent-current").unwrap();
    let previous = usecase
        .arm(slot_id(), ProviderKind::Codex, previous_scope.clone())
        .await
        .unwrap();

    let current = usecase
        .arm(slot_id(), ProviderKind::Codex, current_scope)
        .await
        .unwrap();

    assert_ne!(previous.binding_id(), current.binding_id());
    assert_eq!(provider_event_count(&store, "agent-previous").await, 2);
    assert_eq!(provider_event_count(&store, "agent-current").await, 1);
    let stale_signal = ProviderLifecycleSignal::session_started(
        previous.binding_id(),
        ProviderKind::Codex,
        previous_scope,
        "codex-session-stale",
        None,
    )
    .unwrap();
    assert_eq!(
        usecase
            .receive(previous.slot_id(), previous.capability(), stale_signal)
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Rejected(ProviderLifecycleRejection::BindingExpired)
    );
    assert_eq!(provider_event_count(&store, "agent-previous").await, 2);
    assert_eq!(provider_event_count(&store, "agent-current").await, 1);
}

#[tokio::test]
pub async fn test_providerライフサイクル再試行_outcome_unknown後もdurable_factを重複させない() {
    let (_directory, store, usecase) = setup_persistence_usecase();
    let armed = usecase
        .arm(slot_id(), ProviderKind::Claude, persistence_scope())
        .await
        .unwrap();
    let session_start = || {
        ProviderLifecycleSignal::session_started(
            armed.binding_id(),
            ProviderKind::Claude,
            persistence_scope(),
            "claude-session-unknown",
            Some("provider://claude/unknown"),
        )
        .unwrap()
    };
    store
        .fault_injector()
        .arm_crash_after_commit_before_readback();

    assert_eq!(
        usecase
            .receive(armed.slot_id(), armed.capability(), session_start())
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Applied
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 2);

    assert_eq!(
        usecase
            .receive(armed.slot_id(), armed.capability(), session_start())
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Duplicate
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 2);
}

#[tokio::test]
pub async fn test_providerライフサイクル再試行_outcome_unknownの照会失敗後も同一commitを確定する() {
    let directory = TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
    ))
    .unwrap();
    let repository = Arc::new(ResolveFailureOnceRepository::new(store.clone()));
    let events = LocalProviderLifecycleEventRepository::new(
        crate::usecase::retry::shared().clone(),
        repository as Arc<dyn LocalEventTransactionRepository>,
        store.installation_id().to_string(),
    );
    let usecase = ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(events),
    );
    let armed = usecase
        .arm(slot_id(), ProviderKind::Claude, persistence_scope())
        .await
        .unwrap();
    let session_start = ProviderLifecycleSignal::session_started(
        armed.binding_id(),
        ProviderKind::Claude,
        persistence_scope(),
        "claude-session-resolve-retry",
        Some("provider://claude/resolve-retry"),
    )
    .unwrap();
    store
        .fault_injector()
        .arm_crash_after_commit_before_readback();

    assert_eq!(
        usecase
            .receive(armed.slot_id(), armed.capability(), session_start)
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Applied
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 2);
}

#[tokio::test]
pub async fn test_providerライフサイクル再試行_中断後も同一commitを確定して重複を防ぐ() {
    let directory = TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
    ))
    .unwrap();
    let repository = Arc::new(ResolveFailureRepository::new(store.clone()));
    let events = LocalProviderLifecycleEventRepository::new(
        crate::usecase::retry::shared().clone(),
        repository.clone() as Arc<dyn LocalEventTransactionRepository>,
        store.installation_id().to_string(),
    );
    let usecase = ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(events),
    );
    let armed = usecase
        .arm(slot_id(), ProviderKind::Claude, persistence_scope())
        .await
        .unwrap();
    let session_start = || {
        ProviderLifecycleSignal::session_started(
            armed.binding_id(),
            ProviderKind::Claude,
            persistence_scope(),
            "claude-session-persistent-resolve-failure",
            Some("provider://claude/persistent-resolve-failure"),
        )
        .unwrap()
    };
    repository.set_resolve_failure(true);
    store
        .fault_injector()
        .arm_crash_after_commit_before_readback();

    let first = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        usecase.receive(armed.slot_id(), armed.capability(), session_start()),
    )
    .await;
    assert!(
        first.is_err(),
        "temporary resolution failures remain retryable"
    );

    repository.set_resolve_failure(false);
    assert_eq!(
        usecase
            .receive(armed.slot_id(), armed.capability(), session_start())
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Applied
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 2);
}
