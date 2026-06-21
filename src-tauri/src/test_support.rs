use std::sync::Arc;

use crate::adaptor::gateway::agent_session::FileSessionStorage;
use crate::usecase::agent_session::session::SessionStore;

pub(crate) fn build_session_store() -> SessionStore {
    SessionStore::new(Arc::new(FileSessionStorage::default()))
}
