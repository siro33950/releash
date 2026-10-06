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
#[path = "test_support.rs"]
pub mod test_support;
