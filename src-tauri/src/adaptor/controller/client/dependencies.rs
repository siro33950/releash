use std::sync::Arc;

pub(crate) struct ClientDependencies {
    pub(crate) app_state: Option<crate::adaptor::controller::state::AppState>,
    pub(crate) review_comment_usecase:
        Option<std::sync::Arc<crate::usecase::comment::ReviewCommentUsecase>>,
    pub(crate) data_dir: Result<std::path::PathBuf, String>,
    pub(crate) comment_notify: Arc<crate::adaptor::gateway::push::CommentChangeGateway>,
}
