use crate::domain::failure::TechnicalFailure;

/// provider の設定の版が上限に達し、更新を受け付けられない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderConfigurationRevisionExhausted;

/// 設定の取得・保存または入力値の不正による失敗。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppConfigError {
    Technical(TechnicalFailure),
    InvalidInput(String),
}

impl std::fmt::Display for AppConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Technical(failure) => std::fmt::Display::fmt(failure, f),
            Self::InvalidInput(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for AppConfigError {}
