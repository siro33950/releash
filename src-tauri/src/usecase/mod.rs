pub(crate) mod agent_session;
pub(crate) mod app_config;
pub(crate) mod code_dto;
pub(crate) mod code_error;
pub(crate) mod code_query_service;
pub(crate) mod code_usecase;
pub(crate) mod external_editor;
pub(crate) mod hooks;
pub(crate) mod notification;
pub(crate) mod pty_session;
pub(crate) mod remote_access;
pub(crate) mod repo_paths_usecase;
pub(crate) mod repository_dto;
pub(crate) mod repository_error;
pub(crate) mod repository_query_service;
pub(crate) mod repository_usecase;
pub(crate) mod workspace_state;
// #1034 staged workflow migration: controller wiring switches to this module in #1037.
#[allow(dead_code, unused_imports)]
pub(crate) mod workflow;
