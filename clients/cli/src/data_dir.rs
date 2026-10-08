use std::path::PathBuf;

/// debug / release ビルド種別。
///
/// `cfg!(debug_assertions)` をテスト境界へ閉じ込めるための拡張点。
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

fn protocol_constant(name: &str) -> String {
    crate::descriptor::option(
        &crate::descriptor::client_service().parent_file().options(),
        name,
    )
    .as_str()
    .expect("data directory option")
    .to_owned()
}

pub fn default_data_dir_name_for_profile(profile: BuildProfile) -> &'static str {
    static PRODUCTION: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| protocol_constant("data_dir_name"));
    static DEVELOPMENT: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
            "{}{}",
            *PRODUCTION,
            protocol_constant("development_data_dir_suffix")
        )
    });
    match profile {
        BuildProfile::Production => &PRODUCTION,
        BuildProfile::Development => &DEVELOPMENT,
    }
}

/// 起動環境別の既定 data dir。OS の data dir を解決できないときは `None`。
pub fn default_data_dir_for_profile(profile: BuildProfile) -> Option<PathBuf> {
    dirs::data_dir().map(|base| base.join(default_data_dir_name_for_profile(profile)))
}

pub fn resolve_data_dir(explicit: Option<PathBuf>) -> Result<PathBuf, String> {
    explicit
        .or_else(|| {
            std::env::var_os(protocol_constant("data_dir_env"))
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| default_data_dir_for_profile(BuildProfile::current()))
        .ok_or_else(|| "OS data directory is unavailable".to_string())
}

#[cfg(test)]
#[path = "data_dir_test.rs"]
mod data_dir_tests;
