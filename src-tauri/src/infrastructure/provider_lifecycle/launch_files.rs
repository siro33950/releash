use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub(crate) enum ProviderLaunchFilesError {
    #[error("Provider launch file path is invalid")]
    InvalidPath,
    #[error("Provider launch files are unavailable")]
    Unavailable,
}

pub(crate) fn materialize(
    directory: &Path,
    files: &[(PathBuf, Vec<u8>)],
) -> Result<(), ProviderLaunchFilesError> {
    for (relative_path, _) in files {
        validate_relative_path(relative_path)?;
    }
    std::fs::create_dir_all(directory).map_err(|_| ProviderLaunchFilesError::Unavailable)?;
    for (relative_path, contents) in files {
        let path = directory.join(relative_path);
        let parent = path.parent().ok_or(ProviderLaunchFilesError::InvalidPath)?;
        std::fs::create_dir_all(parent).map_err(|_| ProviderLaunchFilesError::Unavailable)?;
        std::fs::write(path, contents).map_err(|_| ProviderLaunchFilesError::Unavailable)?;
    }
    Ok(())
}

pub(crate) fn cleanup(directory: &Path) -> Result<(), ProviderLaunchFilesError> {
    match std::fs::remove_dir_all(directory) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(ProviderLaunchFilesError::Unavailable),
    }
}

fn validate_relative_path(path: &Path) -> Result<(), ProviderLaunchFilesError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ProviderLaunchFilesError::InvalidPath);
    }
    Ok(())
}
