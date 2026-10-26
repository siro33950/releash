use super::{Error, Thread};
use crate::domain::workspace_tree::WorkspaceIdentity;

/// 指定した Workspace に属する Thread を識別子から復元する Repository。
pub(crate) trait ThreadRepository: Send + Sync {
    /// 指定した Workspace の Thread を取得する。その Workspace に存在しない ID があれば失敗する。
    fn find_by_ids(
        &self,
        workspace: &WorkspaceIdentity,
        thread_ids: &[String],
    ) -> Result<Vec<Thread>, Error>;
}
