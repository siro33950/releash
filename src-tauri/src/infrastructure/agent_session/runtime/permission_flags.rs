//! 抽象 `PermissionMode` → 各CLIバックエンド固有フラグへの変換ヘルパー。
//!
//! 抽象モード自体の語彙・ランクは [`crate::permission::PermissionMode`] が単一の正典として
//! 保持する。CLI固有フラグの語彙（Claude の `default` / `acceptEdits` / `bypassPermissions`、
//! Codex の `sandbox_mode` / `approval_policy`）はバックエンドの実装詳細として
//! このモジュールに閉じ込め、コア型層には露出させない。

use crate::permission::PermissionMode;

/// 抽象モード → Claude SDK の permissionMode 値
pub(super) fn claude_flag_from_mode(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask => "default",
        PermissionMode::Edit => "acceptEdits",
        PermissionMode::Full => "bypassPermissions",
    }
}

/// Claude SDK の permissionMode 値 → 抽象モード
/// SDK が動的に返す "default" / "acceptEdits" / "bypassPermissions" を抽象モードに戻す。
/// "plan" は廃止語彙のため、安全側として None を返す（呼び出し側で無視する）。
pub(super) fn mode_from_claude_flag(flag: &str) -> Option<PermissionMode> {
    match flag {
        "default" => Some(PermissionMode::Ask),
        "acceptEdits" => Some(PermissionMode::Edit),
        "bypassPermissions" => Some(PermissionMode::Full),
        _ => None,
    }
}

/// 抽象モード → Codex CLI の sandbox_mode 値
pub(super) fn codex_sandbox_mode_from_mode(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask => "read-only",
        PermissionMode::Edit => "workspace-write",
        PermissionMode::Full => "danger-full-access",
    }
}

/// 抽象モード → Codex CLI の approval_policy 値
pub(super) fn codex_approval_policy_from_mode(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask => "on-request",
        PermissionMode::Edit => "on-request",
        PermissionMode::Full => "never",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_flag_mapping() {
        assert_eq!(claude_flag_from_mode(PermissionMode::Ask), "default");
        assert_eq!(claude_flag_from_mode(PermissionMode::Edit), "acceptEdits");
        assert_eq!(
            claude_flag_from_mode(PermissionMode::Full),
            "bypassPermissions"
        );
    }

    #[test]
    fn claude_flag_roundtrip() {
        for mode in [
            PermissionMode::Ask,
            PermissionMode::Edit,
            PermissionMode::Full,
        ] {
            assert_eq!(
                mode_from_claude_flag(claude_flag_from_mode(mode)),
                Some(mode)
            );
        }
        assert_eq!(mode_from_claude_flag("plan"), None);
        assert_eq!(mode_from_claude_flag("unknown"), None);
        assert_eq!(mode_from_claude_flag(""), None);
    }

    #[test]
    fn codex_sandbox_mode_mapping() {
        assert_eq!(
            codex_sandbox_mode_from_mode(PermissionMode::Ask),
            "read-only"
        );
        assert_eq!(
            codex_sandbox_mode_from_mode(PermissionMode::Edit),
            "workspace-write"
        );
        assert_eq!(
            codex_sandbox_mode_from_mode(PermissionMode::Full),
            "danger-full-access"
        );
    }

    #[test]
    fn codex_approval_policy_mapping() {
        assert_eq!(
            codex_approval_policy_from_mode(PermissionMode::Ask),
            "on-request"
        );
        assert_eq!(
            codex_approval_policy_from_mode(PermissionMode::Edit),
            "on-request"
        );
        assert_eq!(
            codex_approval_policy_from_mode(PermissionMode::Full),
            "never"
        );
    }
}
