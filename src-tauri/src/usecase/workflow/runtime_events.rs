//! Domain decision to durable event mapping.
//!
//! 実行木の前進イベント（NodeStarted / NodeCompleted / ApprovalRequested /
//! ArtifactProduced / ExecutionCompleted）は domain の集約が直接発行するため、
//! この module は残っていない。abort 系の終端イベントは gateway の各経路が
//! 直接構築する。
