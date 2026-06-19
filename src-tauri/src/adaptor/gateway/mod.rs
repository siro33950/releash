pub(crate) mod code;
pub(crate) mod external_editor;
pub(crate) mod hooks;
pub(crate) mod notification;
pub(crate) mod pty_session;
pub(crate) mod remote_access;
pub(crate) mod repository;
pub(crate) mod shared;
pub(crate) mod workspace_state;
// #1036 staged workflow migration: controller wiring switches to this module in #1037.
#[allow(dead_code, unused_imports)]
pub(crate) mod workflow;
