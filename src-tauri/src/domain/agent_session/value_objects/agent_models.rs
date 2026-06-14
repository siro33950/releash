/// Claude バックエンドで選択可能な固定モデル一覧（表示順）。
/// config.toml / ユーザー編集では変更できない（Rust 定数で完全固定）。
pub(crate) const CLAUDE_FIXED_MODELS: &[&str] = &[
    "claude-opus-4-8",
    "claude-opus-4-7",
    "opus[1m]",
    "claude-sonnet-4-5",
    "claude-haiku-4-5-20251001",
];

/// Codex バックエンドで選択可能な固定モデル一覧（表示順）。
/// config.toml / ユーザー編集では変更できない（Rust 定数で完全固定）。
pub(crate) const CODEX_FIXED_MODELS: &[&str] = &["gpt-5.5", "gpt-5.4", "gpt-5.4-mini"];
