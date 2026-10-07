use std::path::PathBuf;

use super::path_aliases::{default_data_dir_for_profile, BuildProfile};

pub fn resolve_data_dir() -> Result<PathBuf, String> {
    default_data_dir_for_profile(BuildProfile::current())
}
