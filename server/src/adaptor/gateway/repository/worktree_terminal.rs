//! repository ドメインの `WorktreeTerminalGateway` 実装。
//!
//! worktree 削除手順（usecase）が要求する「worktree に紐づく terminal surface の
//! 停止」を terminal surface application の kill ユースケースへ委譲する。停止は
//! best-effort であり、個別の失敗は terminal surface 側でログされ削除手順を
//! 中断させない。

use crate::domain::repository::WorktreeTerminalGateway;
use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;

impl WorktreeTerminalGateway for TerminalSurfaceApplication {
    fn kill_by_worktree(&self, worktree_path: &str) {
        TerminalSurfaceApplication::kill_by_worktree(self, worktree_path);
    }
}

/// terminal runtime を持たない composition（standalone read-only 等）向けの no-op 実装。
pub(crate) struct NoopWorktreeTerminalGateway;

impl WorktreeTerminalGateway for NoopWorktreeTerminalGateway {
    fn kill_by_worktree(&self, _worktree_path: &str) {}
}
