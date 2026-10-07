use std::path::PathBuf;

pub fn resolve_data_dir() -> Result<PathBuf, String> {
    releash_sdk::data_dir::resolve_data_dir(None)
}
