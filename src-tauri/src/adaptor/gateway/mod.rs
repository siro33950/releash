pub(crate) mod agent_session;
pub(crate) mod app_config;
pub(crate) mod app_data_gc;
pub(crate) mod application_lifecycle;
pub(crate) mod code;
pub(crate) mod comment;
#[cfg(feature = "desktop")]
pub(crate) mod daemon_supervision;
#[cfg(feature = "desktop")]
pub(crate) mod desktop_update;
pub(crate) mod external_editor;
pub(crate) mod git_host;
pub(crate) mod local_api;
pub(crate) mod local_event_store;
pub(crate) mod notion;
pub(crate) mod provider_lifecycle;
pub(crate) mod repository;
pub(crate) mod shared;
pub(crate) mod terminal_surface;
pub(crate) mod workflow;
pub(crate) mod workspace_state;
pub(crate) mod workspace_tree;

pub(crate) mod push;

pub(crate) mod telemetry;

#[cfg(feature = "desktop")]
pub(crate) mod login_item;

#[cfg(feature = "desktop")]
pub(crate) mod desktop_client;

#[cfg(feature = "desktop")]
pub(crate) mod cli_install;

pub(crate) mod identity;
pub(crate) mod state_subscription_reads;
pub(crate) mod subscription_timer;
pub(crate) mod work_queue;
