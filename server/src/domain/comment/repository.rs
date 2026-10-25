use super::{Error, Thread};

/// Thread を識別子から復元する Repository。
pub(crate) trait ThreadRepository: Send + Sync {
    /// 指定した Thread を取得する。存在しない ID があれば失敗する。
    fn find_by_ids(&self, thread_ids: &[String]) -> Result<Vec<Thread>, Error>;
}
