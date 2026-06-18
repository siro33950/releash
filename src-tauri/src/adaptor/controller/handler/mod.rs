//! WebSocket ハンドラ（薄い入口）。
//!
//! ルーティング（`ws_server/routing`）から呼ばれ、usecase / query service を
//! 呼び出して response メッセージへ整形する。broadcaster / pty_manager 等の
//! トランスポート連携は引数で受け取る（design.md: routing → controller/handler →
//! usecase / query service）。
pub(crate) mod repository;
pub(crate) mod shared;
pub(crate) mod workflow;
