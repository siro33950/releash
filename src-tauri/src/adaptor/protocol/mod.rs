//! 外部入口（Tauri コマンド引数・WebSocket メッセージ）のメッセージ型。
//!
//! ドメイン型でも DTO でもない（[`CONTROLLER.md`] 参照）。フロントから受け取る転送表現を
//! 受理し、controller が対応するドメイン値オブジェクトへ変換する。

pub(crate) mod code;
pub(crate) mod mention;
pub(crate) mod pty;
pub(crate) mod workflow;
