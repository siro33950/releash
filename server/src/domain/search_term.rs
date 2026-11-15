/// 一覧を絞り込む検索語。前後の空白を除いた 1 文字以上の文字列。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SearchTerm(String);

impl SearchTerm {
    /// 入力から検索語を作る。前後の空白を除き、残りが無ければ検索語ではないので `None` を返す。
    pub fn new(input: &str) -> Option<Self> {
        let trimmed = input.trim();
        (!trimmed.is_empty()).then(|| Self(trimmed.to_owned()))
    }

    /// 文字列が検索語を含むかを決める。大文字と小文字は区別しない。
    pub fn matches(&self, text: &str) -> bool {
        text.to_lowercase().contains(&self.0.to_lowercase())
    }

    /// 検索語の文字列を返す。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
#[path = "search_term_test.rs"]
mod search_term_tests;
