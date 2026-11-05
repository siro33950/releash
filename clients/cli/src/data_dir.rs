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

/// 移行前の既定 data dir の名前。本番は `legacy_data_dir_name`、開発はそれに開発用の接尾辞を付けたもの。
pub fn legacy_data_dir_name_for_profile(profile: BuildProfile) -> &'static str {
    static PRODUCTION: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| protocol_constant("legacy_data_dir_name"));
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

/// 起動環境別の、移行前の既定 data dir。OS の data dir を解決できないときは `None`。
pub fn legacy_data_dir_for_profile(profile: BuildProfile) -> Option<PathBuf> {
    dirs::data_dir().map(|base| base.join(legacy_data_dir_name_for_profile(profile)))
}

/// 解決した data dir。既定の data dir のときだけ、移行前の旧既定の data dir を持つ。
/// 明示指定（`--data-dir`、`RELEASH_DATA_DIR`）は旧 data dir を持たない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDir {
    pub path: PathBuf,
    pub legacy: Option<PathBuf>,
}

/// `--data-dir`、`RELEASH_DATA_DIR`、ビルドの既定値の順で data dir を決める。
pub fn resolve_data_dir(explicit: Option<PathBuf>) -> Result<DataDir, String> {
    let _ = explicit;
    todo!()
}

#[cfg(test)]
#[path = "data_dir_test.rs"]
mod data_dir_tests;
