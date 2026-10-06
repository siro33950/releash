use std::path::PathBuf;

use super::path_aliases::{default_data_dir_for_profile, BuildProfile};

pub fn resolve_data_dir() -> Result<PathBuf, String> {
    default_data_dir_for_profile(BuildProfile::current())
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
