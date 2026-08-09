//! 外部入口（Tauri コマンド引数・イベント payload）のメッセージ型。
//!
//! ドメイン型でも DTO でもない（[`CONTROLLER.md`] 参照）。フロントから受け取る転送表現を
//! 受理し、controller が対応するドメイン値オブジェクトへ変換する。

pub(crate) mod agent;
pub(crate) mod agent_session_notice;
pub(crate) mod agent_session_v1;
pub(crate) mod application_lifecycle_v1;
pub(crate) mod code;
pub(crate) mod mention;
pub(crate) mod notion;
pub(crate) mod provider_agent_session;
pub(crate) mod provider_lifecycle;
pub(crate) mod terminal;
pub(crate) mod workflow;

pub(crate) use agent::*;
