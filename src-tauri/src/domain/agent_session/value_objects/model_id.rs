/// モデルID 1 件の最大コードポイント数。
pub(crate) const MAX_MODEL_ID_LEN: usize = 128;

/// 検証済みモデルID。
///
/// 入力値は変更せず、trim 等の正規化も行わない。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ModelId(String);

impl ModelId {
    /// モデルID の形式検証。
    /// - 空文字列は拒否
    /// - Unicode White_Space のみで構成される文字列は拒否
    /// - 制御文字（U+0000-U+001F, U+007F）を含む文字列は拒否
    /// - コードポイント数が `MAX_MODEL_ID_LEN` を超える場合は拒否
    /// - 入力値は変更しない（trim 等の正規化は行わない）
    pub(crate) fn parse(model_id: impl Into<String>) -> Result<Self, String> {
        let model_id = model_id.into();
        if model_id.is_empty() {
            return Err("モデルIDが空です".to_string());
        }
        if model_id.chars().all(|c| c.is_whitespace()) {
            return Err("モデルIDが空白のみで構成されています".to_string());
        }
        if model_id.chars().any(|c| c.is_control() || c == '\u{007F}') {
            return Err("モデルIDに制御文字が含まれています".to_string());
        }
        if model_id.chars().count() > MAX_MODEL_ID_LEN {
            return Err(format!(
                "モデルIDが上限長 {MAX_MODEL_ID_LEN} を超えています"
            ));
        }
        Ok(Self(model_id))
    }

    #[cfg(test)]
    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_id_rejects_empty() {
        assert!(ModelId::parse("").is_err());
    }

    #[test]
    fn model_id_rejects_whitespace_only() {
        assert!(ModelId::parse("   ").is_err());
        assert!(ModelId::parse("\t\n").is_err());
    }

    #[test]
    fn model_id_preserves_surrounding_whitespace() {
        let input = "  model  ";
        assert_eq!(ModelId::parse(input).unwrap().into_string(), input);
    }

    #[test]
    fn model_id_rejects_unicode_whitespace_only() {
        assert!(ModelId::parse("\u{3000}").is_err());
        assert!(ModelId::parse("\u{00A0}\u{3000}").is_err());
    }

    #[test]
    fn model_id_rejects_control_characters() {
        assert!(ModelId::parse("model\u{0001}id").is_err());
        assert!(ModelId::parse("model\u{007F}id").is_err());
    }

    #[test]
    fn model_id_rejects_too_long() {
        let id: String = "x".repeat(MAX_MODEL_ID_LEN + 1);
        assert!(ModelId::parse(id).is_err());
    }

    #[test]
    fn model_id_accepts_exact_max_length() {
        let id: String = "x".repeat(MAX_MODEL_ID_LEN);
        assert!(ModelId::parse(id).is_ok());
    }
}
