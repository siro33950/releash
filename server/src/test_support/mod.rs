#[cfg(feature = "test-support")]
pub mod agent_tui_fixture;

use std::ffi::{OsStr, OsString};
use std::path::Path;

#[cfg(feature = "test-support")]
pub(crate) mod agent_session;
pub(crate) mod retry;
pub(crate) mod state_subscription;

pub fn client_api_deps(
    dispatch: std::sync::Arc<crate::adaptor::controller::client::ClientCommandDispatch>,
) -> crate::adaptor::controller::api::ClientApiDeps {
    crate::adaptor::controller::api::ClientApiDeps::new(
        dispatch,
        crate::adaptor::controller::daemon::client_priority_interceptor(),
    )
}

pub static TEST_ENV_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

pub struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    pub fn set_value(key: &'static str, value: &str) -> Self {
        Self::set_os(key, OsStr::new(value))
    }

    pub fn set_path(key: &'static str, value: &Path) -> Self {
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

struct CapturingLogger {
    messages: std::sync::Mutex<Vec<(log::Level, String)>>,
}

impl log::Log for CapturingLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            self.messages
                .lock()
                .unwrap()
                .push((record.level(), record.args().to_string()));
        }
    }

    fn flush(&self) {}
}

static CAPTURING_LOGGER: CapturingLogger = CapturingLogger {
    messages: std::sync::Mutex::new(Vec::new()),
};
static CAPTURING_LOGGER_INIT: std::sync::Once = std::sync::Once::new();

pub fn install_capturing_logger() {
    CAPTURING_LOGGER_INIT.call_once(|| {
        log::set_logger(&CAPTURING_LOGGER).unwrap();
        log::set_max_level(log::LevelFilter::Trace);
    });
}

pub fn captured_error_messages() -> Vec<String> {
    CAPTURING_LOGGER
        .messages
        .lock()
        .unwrap()
        .iter()
        .filter(|(level, _)| *level == log::Level::Error)
        .map(|(_, message)| message.clone())
        .collect()
}

pub fn captured_warning_messages() -> Vec<String> {
    CAPTURING_LOGGER
        .messages
        .lock()
        .unwrap()
        .iter()
        .filter(|(level, _)| *level == log::Level::Warn)
        .map(|(_, message)| message.clone())
        .collect()
}

#[cfg(feature = "test-support")]
#[path = "../integration_test_support.rs"]
pub mod integration;

#[cfg(feature = "test-support")]
pub mod agent_session_tui_acceptance {
    pub use crate::agent_session_tui_acceptance::{
        drain_and_close_store, AcceptanceAgentSession, AcceptanceAgentSessionLifecycle,
        AcceptanceAgentSessionTreeLocation, AcceptanceArchiveOutcome, AcceptanceHistoryCandidate,
        AcceptanceHookWarning, AcceptanceProvider, AgentSessionTuiAcceptanceConfig,
        AgentSessionTuiAcceptanceHost,
    };
}

#[cfg(feature = "test-support")]
pub mod client_api_acceptance {
    pub use crate::client_api_acceptance::{
        connect_client, read_current_branch, read_state, request_client, rpc, Branch,
        ClientApiAcceptanceHost, ClientEndpoint, ClientRecoveryAcceptanceHost, ClientRecoveryState,
        DiagnosticReport, DiagnosticSpan, DiagnosticStage, GitError, GitGateway, GitService,
        NativeClient, Severity,
    };
}

#[cfg(feature = "test-support")]
pub mod provider_lifecycle_acceptance {
    pub use crate::provider_lifecycle_acceptance::{
        AcceptanceFact, AcceptanceFactKind, AcceptanceIngressResult, AcceptanceLaunch,
        AcceptanceLedgerEventCounts, AcceptanceProvider, AcceptanceScope,
        ProviderLifecycleAcceptanceHost,
    };
}

#[cfg(feature = "test-support")]
pub mod terminal_subscription_acceptance {
    pub use crate::terminal_subscription_acceptance::{
        TerminalSubscription, TerminalSubscriptionHarness,
    };
}

#[cfg(feature = "test-support")]
pub mod workflow_control_plane_acceptance {
    pub use crate::workflow_control_plane_acceptance::{
        AcceptanceNodeExecution, AcceptanceNodeExecutionStatus, AcceptanceNodeKind,
        AcceptanceWorkflowExecution, AcceptanceWorkflowExecutionStatus,
        AcceptanceWorkspaceNodeStatus, WorkflowControlPlaneAcceptanceHost,
    };
}

#[cfg(feature = "test-support")]
pub mod workflow_delegate_acceptance {
    pub use crate::workflow_delegate_acceptance::{
        AcceptanceIngressResult, AcceptanceLaunch, AcceptanceProvider, NodeExecutionStatus,
        RuntimeNodeExecution, WorkflowDelegateAcceptanceHost,
    };
}

#[cfg(feature = "test-support")]
pub mod terminal_surface {
    pub use crate::terminal_surface::{
        initialize_background_work_for_acceptance, TerminalProcessLaunchV1,
        TerminalSurfaceEventFault, TerminalSurfaceOwnerV1, TerminalSurfaceRuntime,
        TerminalSurfaceStreamItemV1, TerminalSurfaceV1,
    };
}

#[cfg(test)]
pub(crate) fn lock_crash_telemetry() -> std::sync::MutexGuard<'static, ()> {
    crate::infrastructure::telemetry::test_helpers::TEST_LOCK
        .lock()
        .unwrap()
}

pub fn terminal_config(
    command: &str,
) -> std::sync::Arc<dyn crate::domain::app_config::AppConfigRepository> {
    struct Config(String);
    impl crate::domain::app_config::AppConfigRepository for Config {
        fn load(
            &self,
        ) -> Result<crate::domain::app_config::AppConfig, crate::domain::app_config::AppConfigError>
        {
            use crate::domain::app_config::value_objects::*;
            Ok(crate::domain::app_config::AppConfig::new(
                false,
                false,
                vec![],
                String::new(),
                self.0.clone(),
                TelemetryConfig {
                    crash_reporting: false,
                    performance_telemetry: false,
                },
                WorkflowConfig {
                    approval_auto_approve: false,
                },
                std::collections::HashMap::new(),
                std::collections::HashMap::new(),
            ))
        }
        fn save(
            &self,
            _: crate::domain::app_config::AppConfig,
        ) -> Result<(), crate::domain::app_config::AppConfigError> {
            panic!("read-only test config")
        }
    }
    std::sync::Arc::new(Config(command.into()))
}
