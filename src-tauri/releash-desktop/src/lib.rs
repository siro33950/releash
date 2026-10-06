mod adaptor;
mod common;
mod desktop;
mod domain;
mod infrastructure;
mod usecase;
use desktop::application_context;
pub use desktop::run;
#[cfg(debug_assertions)]
pub mod desktop_client_acceptance;
