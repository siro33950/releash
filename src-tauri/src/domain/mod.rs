pub(crate) mod agent_session;
pub(crate) mod app_config;
pub(crate) mod code;
pub(crate) mod comment;
pub(crate) mod external_editor;
pub(crate) mod git_host;
pub(crate) mod hooks;
pub(crate) mod notification;
pub(crate) mod notion;
pub(crate) mod pty_session;
pub(crate) mod remote_access;
pub(crate) mod repository;
pub(crate) mod workspace_state;
// #1031 staged migration: workflow domain ports/value objects are introduced
// before usecase/gateway/controller wiring. Remove this allowance as the new
// workflow stack starts consuming the module in #1032-#1036.
#[allow(dead_code, unused_imports)]
pub(crate) mod workflow;
