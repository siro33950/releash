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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelEntry {
    pub id: String,
    pub display_name: String,
    pub backend: String,
    pub model_id: String,
}

pub(crate) fn model_entry_id(backend: &str, model_id: &str) -> String {
    format!("{backend}:{model_id}")
}

pub(crate) fn model_display_name(backend: &str, model_id: &str) -> String {
    match (backend, model_id) {
        ("claude", "claude-opus-4-8") => "Opus 4.8".to_string(),
        ("claude", "claude-opus-4-7") => "Opus 4.7".to_string(),
        ("claude", "opus[1m]") => "Opus 1m".to_string(),
        ("claude", "claude-sonnet-4-5") => "Sonnet 4.5".to_string(),
        ("claude", "claude-haiku-4-5-20251001") => "Haiku 4.5".to_string(),
        ("codex", "gpt-5.5") => "GPT-5.5".to_string(),
        ("codex", "gpt-5.4") => "GPT-5.4".to_string(),
        ("codex", "gpt-5.4-mini") => "GPT-5.4 Mini".to_string(),
        _ => model_id.to_string(),
    }
}

pub(crate) fn model_entry_for_backend_model(backend: &str, model_id: &str) -> ModelEntry {
    ModelEntry {
        id: model_entry_id(backend, model_id),
        display_name: model_display_name(backend, model_id),
        backend: backend.to_string(),
        model_id: model_id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_entry_uses_backend_scoped_id_and_display_name() {
        let entry = model_entry_for_backend_model("claude", "claude-opus-4-8");

        assert_eq!(entry.id, "claude:claude-opus-4-8");
        assert_eq!(entry.display_name, "Opus 4.8");
        assert_eq!(entry.backend, "claude");
        assert_eq!(entry.model_id, "claude-opus-4-8");
    }

    #[test]
    fn unknown_model_display_name_falls_back_to_model_id() {
        assert_eq!(model_display_name("custom", "vendor-model"), "vendor-model");
    }
}
