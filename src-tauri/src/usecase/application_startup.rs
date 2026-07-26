//! Process-local startup admission authority.
//!
//! This authority deliberately owns no SQLite handle and no normal application
//! state. It records the single fixed-store startup attempt and gates every
//! command before Tauri resolves command-specific managed state.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub(crate) trait ProcessLocalExitPort: Send + Sync {
    fn exit(&self, code: i32);
}

#[cfg(test)]
struct NoopProcessLocalExitPort;

#[cfg(test)]
impl ProcessLocalExitPort for NoopProcessLocalExitPort {
    fn exit(&self, _code: i32) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartupFailureKind {
    StoreInUse,
    StorageUnavailable,
    UnsupportedRuntime,
    UnsupportedStoreVersion,
    InitializationStateInvalid,
    StoreValidationFailed,
    SchemaEvolutionFailed,
}

impl StartupFailureKind {
    pub(crate) fn safe_description(self) -> &'static str {
        match self {
            Self::StoreInUse => "Local data is currently in use by another Releash process.",
            Self::StorageUnavailable => "Local data storage is currently unavailable.",
            Self::UnsupportedRuntime => {
                "This Releash build cannot use the bundled local database runtime."
            }
            Self::UnsupportedStoreVersion => {
                "This Releash build does not support the local data version."
            }
            Self::InitializationStateInvalid => {
                "Local data initialization could not be verified safely."
            }
            Self::StoreValidationFailed => "The local data store could not be verified safely.",
            Self::SchemaEvolutionFailed => {
                "The local data store could not be updated safely during startup."
            }
        }
    }

    pub(crate) fn retry_on_next_launch(self) -> bool {
        matches!(
            self,
            Self::StoreInUse | Self::StorageUnavailable | Self::SchemaEvolutionFailed
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StartupFailure {
    pub(crate) kind: StartupFailureKind,
    pub(crate) safe_description: &'static str,
    pub(crate) correlation_id: String,
    pub(crate) retry_on_next_launch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ApplicationStartupOutcome {
    Ready,
    Failed(StartupFailure),
}

pub(crate) struct ApplicationStartupAuthority {
    outcome: ApplicationStartupOutcome,
    failure_exit: Option<Arc<dyn ProcessLocalExitPort>>,
    exit_dispatched: AtomicBool,
}

impl ApplicationStartupAuthority {
    pub(crate) fn ready() -> Self {
        Self {
            outcome: ApplicationStartupOutcome::Ready,
            failure_exit: None,
            exit_dispatched: AtomicBool::new(false),
        }
    }

    pub(crate) fn failed(
        kind: StartupFailureKind,
        failure_exit: Arc<dyn ProcessLocalExitPort>,
    ) -> Self {
        Self {
            outcome: ApplicationStartupOutcome::Failed(StartupFailure {
                kind,
                safe_description: kind.safe_description(),
                correlation_id: uuid::Uuid::new_v4().to_string(),
                retry_on_next_launch: kind.retry_on_next_launch(),
            }),
            failure_exit: Some(failure_exit),
            exit_dispatched: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub(crate) fn failed_kind(kind: StartupFailureKind) -> Self {
        Self::failed(kind, Arc::new(NoopProcessLocalExitPort))
    }

    pub(crate) fn outcome(&self) -> ApplicationStartupOutcome {
        self.outcome.clone()
    }

    pub(crate) fn normal_admission_ready(&self) -> bool {
        matches!(self.outcome, ApplicationStartupOutcome::Ready)
    }

    pub(crate) fn failed_correlation_id(&self) -> Option<&str> {
        match &self.outcome {
            ApplicationStartupOutcome::Ready => None,
            ApplicationStartupOutcome::Failed(failure) => Some(&failure.correlation_id),
        }
    }

    pub(crate) fn quit_after_failure(&self) -> Result<String, ApplicationUnavailable> {
        let correlation_id = self
            .failed_correlation_id()
            .ok_or(ApplicationUnavailable::ApplicationUnavailable)?
            .to_string();
        let failure_exit = self
            .failure_exit
            .as_ref()
            .ok_or(ApplicationUnavailable::ApplicationUnavailable)?;
        if self
            .exit_dispatched
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            failure_exit.exit(1);
        }
        Ok(correlation_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ApplicationUnavailable {
    ApplicationUnavailable,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[derive(Default)]
    struct RecordingProcessLocalExitPort {
        calls: AtomicUsize,
    }

    impl ProcessLocalExitPort for RecordingProcessLocalExitPort {
        fn exit(&self, code: i32) {
            assert_eq!(code, 1);
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn first_failure_quit_dispatches_exit_once_and_every_retry_joins_it() {
        let exit = Arc::new(RecordingProcessLocalExitPort::default());
        let authority =
            ApplicationStartupAuthority::failed(StartupFailureKind::StoreInUse, exit.clone());
        let ApplicationStartupOutcome::Failed(failure) = authority.outcome() else {
            panic!("failed startup must remain failed");
        };
        assert_eq!(failure.kind, StartupFailureKind::StoreInUse);
        assert!(failure.retry_on_next_launch);
        let started = std::time::Instant::now();
        assert_eq!(
            authority.quit_after_failure().as_deref(),
            Ok(failure.correlation_id.as_str())
        );
        assert_eq!(
            exit.calls.load(Ordering::SeqCst),
            1,
            "the first accepted request must dispatch the process-local exit"
        );
        for _ in 0..32 {
            assert_eq!(
                authority.quit_after_failure().as_deref(),
                Ok(failure.correlation_id.as_str())
            );
            assert_eq!(
                exit.calls.load(Ordering::SeqCst),
                1,
                "same-process retries must join without another exit effect"
            );
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(15));
    }

    #[test]
    fn ready_rejects_startup_failure_quit() {
        assert_eq!(
            ApplicationStartupAuthority::ready().quit_after_failure(),
            Err(ApplicationUnavailable::ApplicationUnavailable)
        );
    }

    #[test]
    fn b071_all_startup_failures_are_closed_safe_and_have_only_launch_retry_semantics() {
        let cases = [
            (StartupFailureKind::StoreInUse, true),
            (StartupFailureKind::StorageUnavailable, true),
            (StartupFailureKind::UnsupportedRuntime, false),
            (StartupFailureKind::UnsupportedStoreVersion, false),
            (StartupFailureKind::InitializationStateInvalid, false),
            (StartupFailureKind::StoreValidationFailed, false),
            (StartupFailureKind::SchemaEvolutionFailed, true),
        ];

        for (kind, retry_on_next_launch) in cases {
            let authority = ApplicationStartupAuthority::failed_kind(kind);
            assert!(!authority.normal_admission_ready());
            let ApplicationStartupOutcome::Failed(failure) = authority.outcome() else {
                panic!("{kind:?} must fail closed");
            };
            assert_eq!(failure.kind, kind);
            assert_eq!(failure.retry_on_next_launch, retry_on_next_launch);
            assert!(uuid::Uuid::parse_str(&failure.correlation_id).is_ok());
            let description = failure.safe_description.to_ascii_lowercase();
            for forbidden in [
                "select ",
                "pragma ",
                "sqlite_",
                ".db",
                "/users/",
                "\\users\\",
                "session",
                "workflow",
            ] {
                assert!(
                    !description.contains(forbidden),
                    "{kind:?} leaked forbidden detail {forbidden:?}"
                );
            }
        }
    }
}
