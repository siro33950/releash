//! repository 責務の WebSocket ハンドラ（薄い入口）。
//!
//! branch 情報・worktree 一覧/選択のリモート入口。query service / usecase を
//! 呼び、broadcaster / PTY runtime gateway 等のトランスポート連携は引数で受け取る。
pub(crate) mod branch;
pub(crate) mod worktree;
