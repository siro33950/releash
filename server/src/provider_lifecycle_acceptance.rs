use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::adaptor::gateway::local_event_store::provider_lifecycle_codec::PROVIDER_LIFECYCLE_EVENT_TYPE;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::provider_lifecycle::{
    LocalProviderLifecycleCredentialGateway, LocalProviderLifecycleEventRepository,
    ProviderLaunchContext, ProviderLaunchSpec,
};
use crate::domain::local_event::{
    LoadStreamRequest, LoadedDomainEvent, LocalDomainEvent, LocalEventTransactionRepository,
    StreamId,
};
use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleEvent, ProviderLifecycleScope, ProviderLifecycleSlotId,
    ProviderLifecycleUnavailableReason,
};
use crate::infrastructure::local_api::LocalApiServer;
use crate::usecase::provider_lifecycle::ProviderLifecycleUsecase;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptanceProvider {
    Claude,
    Codex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptanceIngressResult {
    Applied,
    Ignored,
    Duplicate,
    Rejected { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceScope {
    pub agent_session_id: String,
}

impl AcceptanceScope {
    pub fn new(agent_session_id: impl Into<String>) -> Self {
        Self {
            agent_session_id: agent_session_id.into(),
        }
    }

    fn domain(&self) -> Result<ProviderLifecycleScope, String> {
        ProviderLifecycleScope::new(&self.agent_session_id).map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceLaunchFile {
    pub relative_path: PathBuf,
    pub contents: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceLaunch {
    pub slot_id: String,
    pub binding_id: String,
    pub capability: String,
    pub provider: AcceptanceProvider,
    pub scope: AcceptanceScope,
    pub arguments: Vec<String>,
    pub environment: Vec<(String, String)>,
    pub files: Vec<AcceptanceLaunchFile>,
    pub requires_hook_trust: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptanceFactKind {
    BindingArmed {
        binding_id: String,
        provider: AcceptanceProvider,
        scope: AcceptanceScope,
    },
    SessionAssociated {
        binding_id: String,
        provider_session_id: String,
        transcript_ref: Option<String>,
    },
    TranscriptAssociated {
        binding_id: String,
        transcript_ref: String,
    },
    StopObserved {
        binding_id: String,
    },
    StopFailed {
        binding_id: String,
        reason: String,
    },
    LifecycleUnavailable {
        binding_id: String,
        provider: AcceptanceProvider,
        scope: AcceptanceScope,
        reason: String,
    },
    BindingExpired {
        binding_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceFact {
    pub occurred_at_ms: i64,
    pub kind: AcceptanceFactKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptanceEventCounts {
    pub provider_lifecycle: usize,
    pub other: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptanceLedgerEventCounts {
    pub provider_lifecycle: usize,
    pub other: usize,
}

pub struct ProviderLifecycleAcceptanceHost {
    store: Arc<LocalEventStore>,
    usecase: Arc<ProviderLifecycleUsecase>,
    server: Arc<LocalApiServer>,
    workflow_runtime_command_count: Arc<AtomicUsize>,
}

impl ProviderLifecycleAcceptanceHost {
    pub fn start(data_dir: &Path) -> Result<Self, String> {
        let work = crate::terminal_surface::initialize_background_work_for_acceptance();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            data_dir.to_path_buf(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        ))
        .map_err(|error| error.to_string())?;
        let events = Arc::new(LocalProviderLifecycleEventRepository::new(
            work.retrying.clone(),
            store.clone() as Arc<dyn LocalEventTransactionRepository>,
            store.installation_id().to_string(),
        ));
        let usecase = Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(LocalProviderLifecycleCredentialGateway),
            events,
        ));
        let (binding, daemon) =
            crate::acceptance_test_support::client_binding(data_dir.to_path_buf())
                .map_err(|error| error.to_string())?;
        let workflow_runtime_command_count = Arc::new(AtomicUsize::new(0));
        let mut dispatch = crate::adaptor::controller::client::ClientCommandDispatch::new(daemon);
        for names in [
            &["start_workflow"][..],
            &["abort_workflow"][..],
            &["approve_workflow_node"][..],
            &["workflow_submit_output"][..],
        ] {
            let count = workflow_runtime_command_count.clone();
            dispatch.register_domain(
                names,
                Box::new(move |_| {
                    count.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async {
                        Err(crate::adaptor::controller::client::invalid_request(
                            "workflow runtime command invoked by provider lifecycle",
                        ))
                    })
                }),
            );
        }
        let client = crate::adaptor::controller::api::ClientApiDeps::new(
            Arc::new(dispatch),
            crate::adaptor::controller::daemon::client_priority_interceptor(),
        )
        .with_provider_lifecycle(Arc::new(AcceptancePayloadReceiver(usecase.clone())));
        let router = crate::adaptor::controller::api::build_router(
            crate::adaptor::controller::api::auth::ClientTokens {
                operator: binding.terminal_bearer_token().into(),
                hook: binding.hook_bearer_token(),
            },
            Some(client),
            crate::adaptor::controller::daemon::default_timeout(),
        );
        let server = binding
            .start(router, &tokio::runtime::Handle::current())
            .inspect(|server| {
                server.publish_discovery().unwrap();
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            store,
            usecase,
            server,
            workflow_runtime_command_count,
        })
    }

    pub async fn prepare_launch(
        &self,
        provider: AcceptanceProvider,
        scope: AcceptanceScope,
        claude_plugin_directory: Option<&Path>,
    ) -> Result<AcceptanceLaunch, String> {
        self.prepare_launch_in_slot(
            uuid::Uuid::new_v4().simple().to_string(),
            provider,
            scope,
            claude_plugin_directory,
        )
        .await
    }

    pub async fn prepare_launch_in_slot(
        &self,
        slot_id: impl Into<String>,
        provider: AcceptanceProvider,
        scope: AcceptanceScope,
        claude_plugin_directory: Option<&Path>,
    ) -> Result<AcceptanceLaunch, String> {
        let domain_provider = domain_provider(provider);
        let slot_id =
            ProviderLifecycleSlotId::new(slot_id.into()).map_err(|error| error.to_string())?;
        let armed = self
            .usecase
            .arm(slot_id, domain_provider, scope.domain()?)
            .await
            .map_err(|error| error.to_string())?;
        let context = ProviderLaunchContext::new(
            armed.slot_id().clone(),
            armed.binding_id(),
            armed.capability(),
            armed.scope().clone(),
            "hook-token",
        )
        .map_err(|error| error.to_string())?;
        let hook_cli_alias = crate::infrastructure::platform::path_aliases::alias_name_for_profile(
            crate::infrastructure::platform::path_aliases::BuildProfile::current(),
        );
        let spec = ProviderLaunchSpec::for_provider(
            armed.provider(),
            context,
            hook_cli_alias,
            claude_plugin_directory,
        )
        .map_err(|error| error.to_string())?;
        Ok(AcceptanceLaunch {
            slot_id: armed.slot_id().as_str().to_string(),
            binding_id: armed.binding_id().to_string(),
            capability: armed.capability().to_string(),
            provider,
            scope,
            arguments: spec.arguments().to_vec(),
            environment: spec.environment().to_vec(),
            files: spec
                .files()
                .iter()
                .map(|file| AcceptanceLaunchFile {
                    relative_path: file.relative_path().to_path_buf(),
                    contents: file.contents().to_vec(),
                })
                .collect(),
            requires_hook_trust: spec.requires_hook_trust(),
        })
    }

    pub async fn facts(&self, agent_session_id: &str) -> Result<Vec<AcceptanceFact>, String> {
        let page = self
            .store
            .load_stream(LoadStreamRequest {
                stream_id: StreamId::provider_lifecycle(agent_session_id)
                    .map_err(|error| error.to_string())?,
                after: None,
                limit: 1_024,
            })
            .await
            .map_err(|error| error.to_string())?;
        Ok(page
            .events
            .into_iter()
            .filter_map(|event| {
                let LoadedDomainEvent::Known(inner) = event.event else {
                    return None;
                };
                let LocalDomainEvent::ProviderLifecycle(provider_event) = *inner else {
                    return None;
                };
                Some(AcceptanceFact {
                    occurred_at_ms: event.occurred_at_ms,
                    kind: acceptance_fact(provider_event),
                })
            })
            .collect())
    }

    pub async fn event_counts(
        &self,
        agent_session_id: &str,
    ) -> Result<AcceptanceEventCounts, String> {
        let page = self
            .store
            .load_stream(LoadStreamRequest {
                stream_id: StreamId::provider_lifecycle(agent_session_id)
                    .map_err(|error| error.to_string())?,
                after: None,
                limit: 1_024,
            })
            .await
            .map_err(|error| error.to_string())?;
        let mut counts = AcceptanceEventCounts {
            provider_lifecycle: 0,
            other: 0,
        };
        for event in page.events {
            match event.event {
                LoadedDomainEvent::Known(inner)
                    if matches!(*inner, LocalDomainEvent::ProviderLifecycle(_)) =>
                {
                    counts.provider_lifecycle += 1;
                }
                _ => counts.other += 1,
            }
        }
        Ok(counts)
    }

    pub async fn ledger_event_counts(&self) -> Result<AcceptanceLedgerEventCounts, String> {
        self.store
            .submit_query(|connection| {
                let (all, provider_lifecycle) = connection
                    .query_row(
                        "SELECT COUNT(*),
                                COALESCE(SUM(CASE WHEN event_type = ?1 THEN 1 ELSE 0 END), 0)
                         FROM events",
                        [PROVIDER_LIFECYCLE_EVENT_TYPE],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .map_err(|error| {
                        crate::adaptor::gateway::local_event_store::reader::storage_unavailable(
                            &error,
                        )
                    })?;
                let all = usize::try_from(all).map_err(|_| {
                    crate::domain::local_event::LocalEventQueryError::InvalidRequest
                })?;
                let provider_lifecycle = usize::try_from(provider_lifecycle).map_err(|_| {
                    crate::domain::local_event::LocalEventQueryError::InvalidRequest
                })?;
                Ok(AcceptanceLedgerEventCounts {
                    provider_lifecycle,
                    other: all.saturating_sub(provider_lifecycle),
                })
            })
            .await
            .map_err(|error| error.to_string())
    }

    pub fn workflow_runtime_command_count(&self) -> usize {
        self.workflow_runtime_command_count.load(Ordering::SeqCst)
    }
}

impl Drop for ProviderLifecycleAcceptanceHost {
    fn drop(&mut self) {
        self.server.shutdown();
    }
}

fn domain_provider(provider: AcceptanceProvider) -> ProviderKind {
    match provider {
        AcceptanceProvider::Claude => ProviderKind::Claude,
        AcceptanceProvider::Codex => ProviderKind::Codex,
    }
}

fn acceptance_provider(provider: ProviderKind) -> AcceptanceProvider {
    match provider {
        ProviderKind::Claude => AcceptanceProvider::Claude,
        ProviderKind::Codex => AcceptanceProvider::Codex,
    }
}

fn unavailable_reason(reason: ProviderLifecycleUnavailableReason) -> &'static str {
    match reason {
        ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded => {
            "session_start_deadline_exceeded"
        }
        ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed => {
            "codex_hook_delivery_unconfirmed"
        }
        ProviderLifecycleUnavailableReason::ProviderHookConfigurationRejected => {
            "provider_hook_configuration_rejected"
        }
        ProviderLifecycleUnavailableReason::LocalApiUnavailable => "local_api_unavailable",
    }
}

fn acceptance_scope(scope: ProviderLifecycleScope) -> AcceptanceScope {
    AcceptanceScope {
        agent_session_id: scope.agent_session_id().to_string(),
    }
}

fn acceptance_fact(event: ProviderLifecycleEvent) -> AcceptanceFactKind {
    match event {
        ProviderLifecycleEvent::BindingArmed {
            slot_id: _,
            binding_id,
            provider,
            scope,
        } => AcceptanceFactKind::BindingArmed {
            binding_id,
            provider: acceptance_provider(provider),
            scope: acceptance_scope(scope),
        },
        ProviderLifecycleEvent::SessionAssociated {
            binding_id,
            provider_session_id,
            transcript_ref,
        } => AcceptanceFactKind::SessionAssociated {
            binding_id,
            provider_session_id,
            transcript_ref,
        },
        ProviderLifecycleEvent::TranscriptAssociated {
            binding_id,
            transcript_ref,
        } => AcceptanceFactKind::TranscriptAssociated {
            binding_id,
            transcript_ref,
        },
        ProviderLifecycleEvent::StopObserved { binding_id } => {
            AcceptanceFactKind::StopObserved { binding_id }
        }
        ProviderLifecycleEvent::StopFailed { binding_id, reason } => {
            AcceptanceFactKind::StopFailed { binding_id, reason }
        }
        ProviderLifecycleEvent::LifecycleUnavailable {
            binding_id,
            provider,
            scope,
            reason,
        } => AcceptanceFactKind::LifecycleUnavailable {
            binding_id,
            provider: acceptance_provider(provider),
            scope: acceptance_scope(scope),
            reason: unavailable_reason(reason).to_string(),
        },
        ProviderLifecycleEvent::BindingExpired { binding_id } => {
            AcceptanceFactKind::BindingExpired { binding_id }
        }
    }
}

struct AcceptancePayloadReceiver(Arc<ProviderLifecycleUsecase>);
#[async_trait::async_trait]
impl crate::usecase::provider_lifecycle::ProviderPayloadReceiver for AcceptancePayloadReceiver {
    async fn receive_payload(
        &self,
        slot_id: &crate::domain::provider_lifecycle::ProviderLifecycleSlotId,
        capability: &str,
        input: crate::usecase::provider_lifecycle::ingress::ProviderPayloadInput<'_>,
    ) -> Result<
        (
            crate::domain::provider_lifecycle::ProviderLifecycleIngressResult,
            bool,
        ),
        crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError,
    > {
        use crate::domain::provider_lifecycle::{
            ProviderLifecycleIngressResult, ProviderPayloadInterpretation,
            ProviderPayloadInterpreter,
        };
        use crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError;
        match crate::adaptor::gateway::provider_lifecycle::LocalProviderPayloadInterpreter
            .interpret(input.provider, input.binding_id, input.scope, input.payload)
            .map_err(ProviderLifecycleIngressUsecaseError::Payload)?
        {
            ProviderPayloadInterpretation::Subagent => {
                Ok((ProviderLifecycleIngressResult::Ignored, false))
            }
            ProviderPayloadInterpretation::Signal(signal) => {
                let started = signal.is_session_started();
                self.0
                    .receive(slot_id, capability, signal)
                    .await
                    .map(|result| (result, started))
                    .map_err(Into::into)
            }
        }
    }
}
