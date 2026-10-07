use std::path::PathBuf;

/// debug / release ビルド種別。
///
/// `cfg!(debug_assertions)` をテスト境界へ閉じ込めるための拡張点。
/// 既定の data dir の解決をサーバと CLI で 1 つにするため、ここに置く。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildProfile {
    /// 本番ビルド (release)。
    Production,
    /// dev ビルド (debug)。
    Development,
}

impl BuildProfile {
    /// 現在の cargo ビルド種別から `BuildProfile` を導出する。
    pub fn current() -> Self {
        if cfg!(debug_assertions) {
            Self::Development
        } else {
            Self::Production
        }
    }
}

/// `BuildProfile` から既定の data dir 名（bundle identifier）を決定する。
pub fn default_data_dir_name_for_profile(profile: BuildProfile) -> &'static str {
    match profile {
        BuildProfile::Production => "com.releash.app",
        BuildProfile::Development => "com.releash.app.dev",
    }
}

/// 起動環境別の既定 data dir。OS の data dir を解決できないときは `None`。
pub fn default_data_dir_for_profile(profile: BuildProfile) -> Option<PathBuf> {
    dirs::data_dir().map(|base| base.join(default_data_dir_name_for_profile(profile)))
}

#[cfg(test)]
#[path = "data_dir_test.rs"]
mod data_dir_tests;
