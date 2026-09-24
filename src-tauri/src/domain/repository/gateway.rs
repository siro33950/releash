//! repository ドメインの外部リソース抽象（trait）。
//!
//! 永続化 trait は `repository.rs` に置く。具体実装は
//! `adaptor/gateway/repository/` に配置する。

/// worktree に紐づく terminal surface の停止 port。実装は adaptor/gateway 層が
/// terminal surface 側の kill ユースケースへ委譲する。停止は best-effort であり、
/// 個別の停止失敗は実装側が吸収し、worktree 削除手順を中断させない。
pub trait WorktreeTerminalGateway: Send + Sync {
    fn kill_by_worktree(&self, worktree_path: &str);
}

/// 登録済みリポジトリパスの変更を外部へ通知する port。
pub trait RepoPathsNotifier: Send + Sync {
    fn notify_changed(&self, paths: Vec<String>);
}
