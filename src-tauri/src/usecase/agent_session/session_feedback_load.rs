//! Feedback-supervised session loading (issues-1499 R-011).
//!
//! A load attempt reserves its globally bounded feedback slot before touching
//! the session projection. Both public transports call this usecase, so an
//! unreadable session and unreadable metadata produce one canonical feedback
//! identity even when no session body can be returned.

use std::sync::Arc;

use crate::domain::local_event::{SafeOperationFailure, SessionOperationFailureKind};
use crate::usecase::agent_session::feedback::{FeedbackError, SessionFeedbackUsecase};
use crate::usecase::agent_session::notice_state::AgentSessionNoticeOperation;
use crate::usecase::agent_session::session::GetSessionResponse;

#[cfg(test)]
static SESSION_LOAD_LOG_MESSAGES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn log_session_load_failure(correlation_id: &str) {
    let message =
        format!("session load failed (correlation_id={correlation_id}, operation=load_session)");
    log::warn!("{message}");
    #[cfg(test)]
    SESSION_LOAD_LOG_MESSAGES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(message);
}

#[cfg(test)]
pub(crate) fn session_load_failure_was_logged(correlation_id: &str) -> bool {
    let expected =
        format!("session load failed (correlation_id={correlation_id}, operation=load_session)");
    SESSION_LOAD_LOG_MESSAGES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .any(|message| message == &expected)
}

#[async_trait::async_trait]
pub(crate) trait SessionLoadPort: Send + Sync {
    async fn load_session(&self, session_id: &str) -> Result<Option<GetSessionResponse>, String>;
}

#[async_trait::async_trait]
impl SessionLoadPort for crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase {
    async fn load_session(&self, session_id: &str) -> Result<Option<GetSessionResponse>, String> {
        self.get_session(session_id)
            .await
            .map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionFeedbackLoadError {
    Feedback(FeedbackError),
    LoadFailed { failure: SafeOperationFailure },
}

impl From<FeedbackError> for SessionFeedbackLoadError {
    fn from(value: FeedbackError) -> Self {
        Self::Feedback(value)
    }
}

pub(crate) struct SessionFeedbackLoadUsecase {
    loader: Arc<dyn SessionLoadPort>,
    feedback: Arc<SessionFeedbackUsecase>,
}

impl SessionFeedbackLoadUsecase {
    pub(crate) fn new(
        loader: Arc<dyn SessionLoadPort>,
        feedback: Arc<SessionFeedbackUsecase>,
    ) -> Self {
        Self { loader, feedback }
    }

    pub(crate) async fn get_session(
        &self,
        session_id: &str,
        attempt_id: &str,
    ) -> Result<Option<GetSessionResponse>, SessionFeedbackLoadError> {
        let reservation = self
            .feedback
            .reserve_attempt(
                session_id,
                AgentSessionNoticeOperation::LoadSession,
                attempt_id,
            )
            .await?;

        match self.loader.load_session(session_id).await {
            Ok(response) => {
                self.feedback.complete_success(&reservation).await?;
                Ok(response)
            }
            Err(_) => {
                // The correlation identity is stable for this exact caller
                // attempt. A lost materialization reply can therefore replay
                // byte-for-byte without leaking the raw storage error.
                let correlation_id = format!("session-load-{}", reservation.feedback_id);
                log_session_load_failure(&correlation_id);
                let failure = SafeOperationFailure::new(
                    SessionOperationFailureKind::PersistFailure,
                    true,
                    "The session could not be loaded.",
                    correlation_id,
                )
                .with_detail("Retry loading the session or dismiss this feedback.");
                self.feedback
                    .materialize_failure(&reservation, failure.clone(), None)
                    .await?;
                Err(SessionFeedbackLoadError::LoadFailed { failure })
            }
        }
    }
}
