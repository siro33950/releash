use super::*;
use crate::common::operation_context::OperationStopped;
use crate::infrastructure::process::output::ProcessError;

#[test]
fn test_プロセス失敗_ioの文と停止の種類を保つ() {
    // Given / When
    let io = GitError::from(ProcessError::Io(std::io::Error::other("spawn failed")));
    // Then
    assert_eq!(io, GitError::External("spawn failed".into()));
    for stopped in [OperationStopped::Cancelled, OperationStopped::Expired] {
        assert_eq!(
            GitError::from(ProcessError::Stopped(stopped)),
            GitError::Technical(stopped.into())
        );
    }
}
