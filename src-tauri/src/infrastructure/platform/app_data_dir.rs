use std::path::PathBuf;

use super::path_aliases::{default_data_dir_for_profile, BuildProfile};

pub(crate) fn resolve_data_dir() -> Result<PathBuf, String> {
    let override_path = if cfg!(feature = "performance") {
        std::env::var("RELEASH_PERFORMANCE_DATA_DIR").ok()
    } else {
        None
    };
    resolve_for_profile(BuildProfile::application(), override_path)
}

pub(crate) fn resolve_for_profile(
    profile: BuildProfile,
    override_path: Option<String>,
) -> Result<PathBuf, String> {
    match override_path.filter(|value| !value.is_empty()) {
        Some(path) => Ok(PathBuf::from(path)),
        None => default_data_dir_for_profile(profile),
    }
}

#[cfg(test)]
#[path = "app_data_dir_test.rs"]
mod app_data_dir_tests;
