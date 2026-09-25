pub(crate) mod agent_session;
pub(crate) mod app_config;
pub(crate) mod app_data_gc;
pub(crate) mod application_lifecycle;
pub(crate) mod code;
pub(crate) mod comment;
#[cfg(any(test, feature = "desktop"))]
pub(crate) mod daemon_supervision;
pub(crate) mod external_editor;
pub(crate) mod failure;
pub(crate) mod git_host;
pub(crate) mod local_api_discovery;
pub(crate) mod local_event;
pub(crate) mod notion;
pub(crate) mod path;
pub(crate) mod provider_lifecycle;
pub(crate) mod repository;
pub(crate) mod shell;
pub(crate) mod terminal_surface;
pub(crate) mod workflow;
pub(crate) mod workspace_state;
pub(crate) mod workspace_tree;

#[cfg(feature = "desktop")]
pub(crate) mod login_item;

pub(crate) mod failure_records;
pub(crate) mod retry;
pub(crate) mod state_subscription;
pub(crate) mod work_queue;

pub mod operation_context;
