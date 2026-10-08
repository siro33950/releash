pub(crate) mod agent_session;
pub(crate) mod app_config;
pub(crate) mod application_lifecycle;
pub(crate) mod code;
pub(crate) mod comment;
pub(crate) mod dependencies;
pub(crate) mod dispatch;
pub(crate) mod external_editor;
pub(crate) mod git_host;
pub(crate) mod notion;
pub(crate) mod repository;
pub(crate) mod telemetry;
pub(crate) mod terminal_surface;
pub(crate) mod workflow;
pub(crate) mod workspace_state;
pub(crate) mod workspace_tree;
pub(crate) use crate::adaptor::presenter::client::{outcome, value};
pub(crate) use dependencies::ClientDependencies;
pub(crate) use dispatch::{convert, invalid_request, optional, required, ClientCommandDispatch};

pub(crate) mod worktree_mutation;

pub(crate) mod installation;
