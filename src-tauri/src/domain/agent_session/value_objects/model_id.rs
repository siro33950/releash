/// モデルID 1 件の最大コードポイント数。
pub(crate) const MAX_MODEL_ID_LEN: usize = 128;

/// 1 バックエンドあたりに保持するユニークなモデルIDの最大件数。
pub(crate) const MAX_MODELS_PER_BACKEND: usize = 256;

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

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

/// 検証済みかつ重複除去済みのバックエンド別モデルID一覧。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelIdList(Vec<ModelId>);

impl ModelIdList {
    /// モデルID 配列の検証。
    /// - 各要素が `ModelId::parse` を満たす場合のみ受け入れる
    /// - 重複は除去（最初に現れた順を保持）
    /// - 重複除去後のユニーク件数が `MAX_MODELS_PER_BACKEND` を超える場合は拒否
    /// - 検証に失敗した場合は Err を返し、呼び出し元は config を更新してはならない
    pub(crate) fn parse_many(ids: &[String]) -> Result<Self, String> {
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        for id in ids {
            let parsed = ModelId::parse(id.clone())?;
            if seen.insert(parsed.as_str().to_string()) {
                result.push(parsed);
            }
        }
        if result.len() > MAX_MODELS_PER_BACKEND {
            return Err(format!(
                "モデルID件数が上限 {MAX_MODELS_PER_BACKEND} を超えています: {}",
                result.len()
            ));
        }
        Ok(Self(result))
    }

    pub(crate) fn into_strings(self) -> Vec<String> {
        self.0.into_iter().map(ModelId::into_string).collect()
    }
}

pub(crate) fn escaped_for_log(value: &str) -> String {
    format!("{value:?}")
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

    #[test]
    fn model_id_list_removes_duplicates_preserving_order() {
        let input = vec![
            "a".to_string(),
            "b".to_string(),
            "a".to_string(),
            "c".to_string(),
            "b".to_string(),
        ];
        let out = ModelIdList::parse_many(&input).unwrap().into_strings();
        assert_eq!(out, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    }

    #[test]
    fn model_id_list_accepts_duplicate_heavy_input_that_collapses() {
        let many: Vec<String> = (0..MAX_MODELS_PER_BACKEND)
            .map(|i| format!("m{i}"))
            .chain(std::iter::repeat_n("m0".to_string(), 10))
            .collect();
        let out = ModelIdList::parse_many(&many).unwrap().into_strings();
        assert_eq!(out.len(), MAX_MODELS_PER_BACKEND);
    }

    #[test]
    fn model_id_list_rejects_too_many_unique() {
        let many: Vec<String> = (0..(MAX_MODELS_PER_BACKEND + 1))
            .map(|i| format!("m{i}"))
            .collect();
        assert!(ModelIdList::parse_many(&many).is_err());
    }

    #[test]
    fn escaped_for_log_escapes_control_characters() {
        assert_eq!(escaped_for_log("bad\nmodel"), "\"bad\\nmodel\"");
    }
}
