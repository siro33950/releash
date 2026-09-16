use crate::adaptor::gateway::push::BackendPush;

use crate::domain::repository::RepoPathsNotifier;

pub struct RepoPathsNotifyGateway {
    sink: std::sync::Arc<crate::infrastructure::push::PushSink>,
}

impl RepoPathsNotifyGateway {
    pub fn new(sink: std::sync::Arc<crate::infrastructure::push::PushSink>) -> Self {
        Self { sink }
    }
}

impl RepoPathsNotifier for RepoPathsNotifyGateway {
    fn notify_changed(&self, paths: Vec<String>) {
        BackendPush::RepoPathsChanged(&paths).emit(&self.sink);
    }
}
