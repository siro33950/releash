use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::domain::agent_session::aggregates::AgentSessionMutationOutcome;
use crate::domain::agent_session::repository::AgentSessionRepository;
use crate::domain::agent_session::{
    should_read_provider_session_title, ProviderSessionTitleGateway, ProviderSessionTitleRequest,
};

use super::AgentSessionChangeNotifier;

pub(crate) struct ProviderSessionTitleIngestionUsecase {
    repository: Arc<dyn AgentSessionRepository>,
    title_gateway: Arc<dyn ProviderSessionTitleGateway>,
    change_notifier: Arc<dyn AgentSessionChangeNotifier>,
    tick: AtomicU64,
    pub(super) queue: Arc<crate::usecase::work_queue::WorkQueueUsecase>,
}

#[derive(Default)]
struct TitleAttempt {
    session: Option<crate::domain::agent_session::repository::VersionedAgentSession>,
    observed: bool,
    request_id: Option<String>,
}

impl ProviderSessionTitleIngestionUsecase {
    pub(crate) fn new(
        queue: std::sync::Arc<crate::usecase::work_queue::WorkQueueUsecase>,
        repository: Arc<dyn AgentSessionRepository>,
        title_gateway: Arc<dyn ProviderSessionTitleGateway>,
        change_notifier: Arc<dyn AgentSessionChangeNotifier>,
    ) -> Self {
        Self {
            repository,
            title_gateway,
            change_notifier,
            tick: AtomicU64::new(0),
            queue,
        }
    }

    pub(crate) async fn start(self: &Arc<Self>) {
        let this = self.clone();
        self.queue
            .enqueue(
                crate::usecase::work_queue::WorkKey::new("provider_session_title_list", "daemon"),
                crate::common::retry::RetryBackoff::ITEM,
                Arc::new(move |_| {
                    let this = this.clone();
                    Box::pin(async move {
                        this.enqueue_due().await?;
                        Ok(Some(
                            crate::domain::agent_session::PROVIDER_SESSION_TITLE_TICK_INTERVAL,
                        ))
                    })
                }),
            )
            .await;
    }

    async fn enqueue_due(self: &Arc<Self>) -> Result<(), crate::usecase::work_queue::WorkFailure> {
        use crate::usecase::work_queue::{WorkFailure, WorkKey};
        let tick = self.tick.fetch_add(1, Ordering::Relaxed);
        let sessions = self
            .repository
            .list_open_for_provider_session_title()
            .await
            .map_err(|error| WorkFailure::from_error(&error))?;
        for session in sessions {
            if !should_read_provider_session_title(
                tick,
                session.session().provider_session_title().is_some(),
            ) {
                continue;
            }
            let id = session.session().id().to_string();
            let key = WorkKey::new("provider_session_title", &id);
            let this = self.clone();
            let progress = Arc::new(tokio::sync::Mutex::new(TitleAttempt::default()));
            self.queue
                .enqueue(
                    key,
                    crate::common::retry::RetryBackoff::ITEM,
                    Arc::new(move |action| {
                        let this = this.clone();
                        let id = id.clone();
                        let progress = progress.clone();
                        Box::pin(async move {
                            let mut progress = progress.lock().await;
                            if action == crate::usecase::work_queue::AttemptProgress::Reload {
                                *progress = TitleAttempt::default();
                            }
                            this.ingest_session(&id, &mut progress).await?;
                            Ok(None)
                        })
                    }),
                )
                .await;
        }
        Ok(())
    }

    async fn ingest_session(
        &self,
        id: &str,
        progress: &mut TitleAttempt,
    ) -> Result<(), crate::usecase::work_queue::WorkFailure> {
        use crate::usecase::work_queue::WorkFailure;
        if progress.session.is_none() {
            progress.session = self
                .repository
                .find(id)
                .await
                .map_err(|error| WorkFailure::from_error(&error))?;
        }
        let Some(session) = progress.session.as_mut() else {
            return Ok(());
        };
        if !progress.observed {
            let Some(provider_session_id) = session.session().provider_session_id() else {
                return Ok(());
            };
            let request = ProviderSessionTitleRequest {
                provider: session.session().provider(),
                provider_session_id: provider_session_id.to_string(),
                worktree_path: session.session().worktree_path().to_string(),
                transcript_ref: session.session().transcript_ref().map(str::to_string),
            };
            let Some(title) = self
                .title_gateway
                .read_title(request)
                .await
                .map_err(|error| WorkFailure::from_error(&error))?
            else {
                return Ok(());
            };
            let outcome = session
                .session_mut()
                .observe_provider_session_title(title)
                .map_err(|error| WorkFailure::from_error(&error))?;
            if outcome == AgentSessionMutationOutcome::AlreadyApplied {
                return Ok(());
            }
            progress.observed = true;
        }
        let worktree = session.session().workspace().as_str().to_string();
        let request_id = progress.request_id.get_or_insert_with(|| {
            format!(
                "provider-session-title-ingestion.{id}.{}",
                uuid::Uuid::new_v4()
            )
        });
        self.repository
            .save_provider_session_title(session.clone(), request_id)
            .await
            .map_err(|error| WorkFailure::from_error(&error))?;
        self.change_notifier.agent_session_changed(&worktree);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn ingest_due(&self) {
        let tick = self.tick.fetch_add(1, Ordering::Relaxed);
        if let Ok(sessions) = self.repository.list_open_for_provider_session_title().await {
            for session in sessions {
                if should_read_provider_session_title(
                    tick,
                    session.session().provider_session_title().is_some(),
                ) {
                    let _ = self
                        .ingest_session(session.session().id(), &mut TitleAttempt::default())
                        .await;
                }
            }
        }
    }
}
