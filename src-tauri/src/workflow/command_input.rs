//! [04] / [06] `WorkflowCommand` 入力境界のドメインバリデーション。
//!
//! `ApproveNode.comment` / `RejectNode.reason` / approval chat instruction
//! 等、workflow 外部入口で受け取る自由記述テキストに共通する境界ルール
//! （最大文字数、reject reason の非空必須）を engine domain の pure helper
//! に集約する。CLI 入口・dispatcher adapter・engine の各層は同じ helper を
//! 呼び、それぞれの Error 型へ map することで「同一ドメインルールが層を
//! またいで重複する」状態を解消する（review R2-01 凝集対応）。

use std::fmt;

/// approval / reject 系自由記述テキストの最大文字数。
///
/// command 境界での新規外部入力の境界バリデーションに用いる（spec [04]）。
pub(crate) const MAX_APPROVAL_COMMENT_CHARS: usize = 8192;

/// `command_input` helper の検証失敗結果。
///
/// `label` は呼び出し側が制御する人間可読ラベル（例: `"Reject comment"` /
/// `"--reason"` / `"approval chat instruction"`）。各層は自分の Error 型
/// （`WorkflowEngineError::ValidationError` / `CliError::InvalidInput` 等）
/// に変換して伝播させる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommandInputError {
    /// 必須の自由記述テキストが trim 後に空だった。
    Empty { label: &'static str },
    /// 自由記述テキストが最大文字数を超えた。
    TooLong { label: &'static str, limit: usize },
}

impl fmt::Display for CommandInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty { label } => write!(f, "{label} must not be empty"),
            Self::TooLong { label, limit } => write!(f, "{label} exceeds {limit} characters"),
        }
    }
}

impl std::error::Error for CommandInputError {}

/// 任意の自由記述コメント（`ApproveNode.comment` 等）の長さを検証する。
///
/// `None` または `MAX_APPROVAL_COMMENT_CHARS` 文字以下なら OK。空文字
/// （`Some("")`）も許容する（reject reason 以外は空コメントを許容する仕様）。
pub(crate) fn validate_optional_comment_text(
    value: Option<&str>,
    label: &'static str,
) -> Result<(), CommandInputError> {
    if let Some(text) = value {
        validate_text_length(text, label)?;
    }
    Ok(())
}

/// Reject reason 用テキストを検証する。
///
/// trim 後非空 + `MAX_APPROVAL_COMMENT_CHARS` 文字以下なら OK。
pub(crate) fn validate_reject_reason_text(
    value: &str,
    label: &'static str,
) -> Result<(), CommandInputError> {
    if value.trim().is_empty() {
        return Err(CommandInputError::Empty { label });
    }
    validate_text_length(value, label)?;
    Ok(())
}

fn validate_text_length(value: &str, label: &'static str) -> Result<(), CommandInputError> {
    if value.chars().count() > MAX_APPROVAL_COMMENT_CHARS {
        return Err(CommandInputError::TooLong {
            label,
            limit: MAX_APPROVAL_COMMENT_CHARS,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_optional_comment_text_accepts_none() {
        assert!(validate_optional_comment_text(None, "Approve comment").is_ok());
    }

    #[test]
    fn validate_optional_comment_text_accepts_empty_string() {
        assert!(validate_optional_comment_text(Some(""), "Approve comment").is_ok());
    }

    #[test]
    fn validate_optional_comment_text_accepts_max_length() {
        let max_text = "a".repeat(MAX_APPROVAL_COMMENT_CHARS);
        assert!(validate_optional_comment_text(Some(&max_text), "Approve comment").is_ok());
    }

    #[test]
    fn validate_optional_comment_text_rejects_over_max_length() {
        let over = "a".repeat(MAX_APPROVAL_COMMENT_CHARS + 1);
        let err = validate_optional_comment_text(Some(&over), "Approve comment").unwrap_err();
        assert_eq!(
            err,
            CommandInputError::TooLong {
                label: "Approve comment",
                limit: MAX_APPROVAL_COMMENT_CHARS,
            }
        );
        assert_eq!(err.to_string(), "Approve comment exceeds 8192 characters");
    }

    #[test]
    fn validate_reject_reason_text_accepts_normal_input() {
        assert!(validate_reject_reason_text("Please fix the bug", "Reject comment").is_ok());
    }

    #[test]
    fn validate_reject_reason_text_rejects_empty_string() {
        let err = validate_reject_reason_text("", "Reject comment").unwrap_err();
        assert_eq!(
            err,
            CommandInputError::Empty {
                label: "Reject comment"
            }
        );
        assert_eq!(err.to_string(), "Reject comment must not be empty");
    }

    #[test]
    fn validate_reject_reason_text_rejects_whitespace_only() {
        let err = validate_reject_reason_text("   \n\t ", "Reject comment").unwrap_err();
        assert_eq!(
            err,
            CommandInputError::Empty {
                label: "Reject comment"
            }
        );
    }

    #[test]
    fn validate_reject_reason_text_accepts_max_length() {
        let max_text = "a".repeat(MAX_APPROVAL_COMMENT_CHARS);
        assert!(validate_reject_reason_text(&max_text, "Reject comment").is_ok());
    }

    #[test]
    fn validate_reject_reason_text_rejects_over_max_length() {
        let over = "a".repeat(MAX_APPROVAL_COMMENT_CHARS + 1);
        let err = validate_reject_reason_text(&over, "Reject comment").unwrap_err();
        assert_eq!(
            err,
            CommandInputError::TooLong {
                label: "Reject comment",
                limit: MAX_APPROVAL_COMMENT_CHARS,
            }
        );
    }

    /// CLI ラベルでの表示メッセージが既存 CLI エラー文言と一致する。
    #[test]
    fn cli_style_label_produces_cli_message() {
        let err_empty = validate_reject_reason_text("", "--reason").unwrap_err();
        assert_eq!(err_empty.to_string(), "--reason must not be empty");

        let over = "a".repeat(MAX_APPROVAL_COMMENT_CHARS + 1);
        let err_over = validate_optional_comment_text(Some(&over), "--reason").unwrap_err();
        assert_eq!(err_over.to_string(), "--reason exceeds 8192 characters");
    }
}
