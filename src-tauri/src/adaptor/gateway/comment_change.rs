use crate::usecase::state_subscription::{StateChangeSource, StateSubscriptionOutputRef};

pub(crate) struct CommentChangeGateway {
    publisher: StateSubscriptionOutputRef,
}
impl CommentChangeGateway {
    pub(crate) fn new(publisher: StateSubscriptionOutputRef) -> Self {
        Self { publisher }
    }
    pub(crate) fn notify(&self, worktree: &str) {
        self.publisher
            .invalidate(StateChangeSource::ReviewComments(Some(worktree.into())));
    }
}
