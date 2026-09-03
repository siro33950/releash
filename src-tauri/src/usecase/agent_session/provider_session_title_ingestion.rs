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
}

impl ProviderSessionTitleIngestionUsecase {
    pub(crate) fn new(
        repository: Arc<dyn AgentSessionRepository>,
        title_gateway: Arc<dyn ProviderSessionTitleGateway>,
        change_notifier: Arc<dyn AgentSessionChangeNotifier>,
    ) -> Self {
        Self {
            repository,
            title_gateway,
            change_notifier,
            tick: AtomicU64::new(0),
        }
    }

    pub(crate) async fn ingest_due(&self) {
        let tick = self.tick.fetch_add(1, Ordering::Relaxed);
        let sessions = match self.repository.list_open_for_provider_session_title().await {
            Ok(sessions) => sessions,
            Err(error) => {
                log::warn!("provider session title candidate read failed: {error:?}");
                return;
            }
        };
        for mut session in sessions {
            if !should_read_provider_session_title(
                tick,
                session.session().provider_session_title().is_some(),
            ) {
                continue;
            }
            let Some(provider_session_id) = session.session().provider_session_id() else {
                log::warn!(
                    "provider session title candidate {} has no provider session id",
                    session.session().id()
                );
                continue;
            };
            let request = ProviderSessionTitleRequest {
                provider: session.session().provider(),
                provider_session_id: provider_session_id.to_string(),
                worktree_path: session.session().worktree_path().to_string(),
                transcript_ref: session.session().transcript_ref().map(str::to_string),
            };
            let title = match self.title_gateway.read_title(request).await {
                Ok(Some(title)) => title,
                Ok(None) => continue,
                Err(error) => {
                    log::warn!(
                        "provider session title read failed for {}: {error:?}",
                        session.session().id()
                    );
                    continue;
                }
            };
            let outcome = match session.session_mut().observe_provider_session_title(title) {
                Ok(outcome) => outcome,
                Err(error) => {
                    log::warn!(
                        "provider session title observation failed for {}: {error:?}",
                        session.session().id()
                    );
                    continue;
                }
            };
            if outcome == AgentSessionMutationOutcome::AlreadyApplied {
                continue;
            }
            let worktree_path = session.session().worktree_path().to_string();
            let session_id = session.session().id().to_string();
            let caller_request_id = format!(
                "provider-session-title-ingestion.{session_id}.{}",
                uuid::Uuid::new_v4()
            );
            match self
                .repository
                .save_provider_session_title(session, &caller_request_id)
                .await
            {
                Ok(_) => self.change_notifier.agent_session_changed(&worktree_path),
                Err(error) => {
                    log::warn!("provider session title save failed for {session_id}: {error:?}")
                }
            }
        }
    }
}
