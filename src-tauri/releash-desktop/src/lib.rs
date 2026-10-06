mod adaptor;
mod common;
mod desktop;
mod domain;
mod infrastructure;
mod usecase;
#[cfg(any(test, feature = "test-support"))]
use desktop::application_context;
pub use desktop::run;
#[cfg(feature = "test-support")]
mod desktop_client_acceptance;

#[cfg(feature = "test-support")]
pub mod test_support {
    pub use crate::desktop_client_acceptance::*;
    #[cfg(unix)]
    pub mod cli_install {
        pub use crate::infrastructure::platform::cli_install::{
            install_cli_symlink_with_runner, CliInstallStatus,
        };
    }
    pub mod integration {
        include!("integration_test_support.rs");
    }
}
