use std::ffi::OsString;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::sync::Arc;
use std::sync::RwLock;

use crate::domain::agent_session::aggregates::{
    ProviderAvailability, ProviderExecutable, ProviderUnavailableReason, ResolvedProviderExecutable,
};
use crate::domain::agent_session::{
    ProviderExecutableProbeGateway, ProviderExecutableProbeGatewayError,
};
use crate::infrastructure::process::executable_probe::ExecutableProbeResult;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::infrastructure::process::search_path::{LoginShellSearchPathSource, SearchPathSource};

pub struct LocalProviderExecutableProbeGateway {
    search_path: RwLock<SearchPathState>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    search_path_source: Arc<dyn SearchPathSource>,
}

struct SearchPathState {
    value: Option<OsString>,
    complete: bool,
}

impl LocalProviderExecutableProbeGateway {
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub(crate) fn new() -> Self {
        Self {
            search_path: RwLock::new(SearchPathState {
                value: std::env::var_os("PATH"),
                complete: true,
            }),
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            search_path_source: Arc::new(LoginShellSearchPathSource),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_search_path(search_path: Option<OsString>) -> Self {
        Self {
            search_path: RwLock::new(SearchPathState {
                value: search_path,
                complete: true,
            }),
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            search_path_source: Arc::new(LoginShellSearchPathSource),
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub fn with_initial_search_path(
        search_path: Result<
            OsString,
            crate::infrastructure::process::search_path::LoginShellPathError,
        >,
    ) -> Self {
        let (value, complete) = match search_path {
            Ok(search_path) => (Some(search_path), true),
            Err(_) => (std::env::var_os("PATH"), false),
        };
        Self {
            search_path: RwLock::new(SearchPathState { value, complete }),
            search_path_source: Arc::new(LoginShellSearchPathSource),
        }
    }

    #[cfg(all(
        any(test, feature = "test-support"),
        any(target_os = "macos", target_os = "linux")
    ))]
    pub fn with_search_path_source(
        search_path: Option<OsString>,
        search_path_source: Arc<dyn SearchPathSource>,
    ) -> Self {
        Self {
            search_path: RwLock::new(SearchPathState {
                value: search_path,
                complete: true,
            }),
            search_path_source,
        }
    }
}

impl ProviderExecutableProbeGateway for LocalProviderExecutableProbeGateway {
    fn resolve(&self, executable: &ProviderExecutable) -> ProviderAvailability {
        let search_path = match self.search_path.read() {
            Ok(search_path) => search_path,
            Err(_) => {
                return ProviderAvailability::unavailable(ProviderUnavailableReason::ProbeFailed);
            }
        };
        match crate::infrastructure::process::executable_probe::resolve_executable(
            executable.as_str(),
            search_path.value.as_deref(),
        ) {
            ExecutableProbeResult::Resolved(path) => match ResolvedProviderExecutable::new(path) {
                Ok(executable) => ProviderAvailability::available(executable),
                Err(_) => ProviderAvailability::unavailable(ProviderUnavailableReason::ProbeFailed),
            },
            ExecutableProbeResult::NotFound
                if !search_path.complete
                    && std::path::Path::new(executable.as_str())
                        .components()
                        .count()
                        == 1 =>
            {
                ProviderAvailability::unavailable(ProviderUnavailableReason::SearchPathUnavailable)
            }
            ExecutableProbeResult::NotFound => {
                ProviderAvailability::unavailable(ProviderUnavailableReason::NotFound)
            }
            ExecutableProbeResult::NotExecutable => {
                ProviderAvailability::unavailable(ProviderUnavailableReason::NotExecutable)
            }
            ExecutableProbeResult::SearchPathUnavailable => {
                ProviderAvailability::unavailable(ProviderUnavailableReason::SearchPathUnavailable)
            }
            ExecutableProbeResult::ProbeFailed => {
                ProviderAvailability::unavailable(ProviderUnavailableReason::ProbeFailed)
            }
        }
    }

    fn refresh_search_path(&self) -> Result<(), ProviderExecutableProbeGatewayError> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let refreshed = self
            .search_path_source
            .load()
            .map_err(|error| {
                ProviderExecutableProbeGatewayError::Technical(
                    crate::domain::failure::TechnicalFailure {
                        nature: match error {
                            crate::infrastructure::process::search_path::LoginShellPathError::Timeout => {
                                crate::domain::failure::TechnicalFailureNature::TimedOut
                            }
                            _ => crate::domain::failure::TechnicalFailureNature::Other,
                        },
                        message: format!("{error:?}"),
                    },
                )
            })?;
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let refreshed = std::env::var_os("PATH").ok_or_else(|| {
            ProviderExecutableProbeGatewayError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "PATH is not set".into(),
                },
            )
        })?;
        let mut search_path = self.search_path.write().map_err(|error| {
            ProviderExecutableProbeGatewayError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: error.to_string(),
                },
            )
        })?;
        *search_path = SearchPathState {
            value: Some(refreshed),
            complete: true,
        };
        Ok(())
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
#[path = "provider_availability_gateway_test.rs"]
mod provider_availability_gateway_tests;

#[cfg(any(test, feature = "test-support"))]
#[path = "test_helpers_provider_availability_gateway.rs"]
pub(crate) mod shared_test_helpers;
