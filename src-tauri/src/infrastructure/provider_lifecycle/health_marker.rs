use std::path::{Component, Path};

const MAX_MARKER_BYTES: u64 = 4 * 1024;

pub(crate) struct RawProviderHookHealthFailure {
    pub(crate) contents: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ProviderHookHealthMarkerError {
    #[error("Provider Hook health marker path is invalid")]
    InvalidPath,
    #[error("Provider Hook health marker is unavailable")]
    Unavailable,
}

pub(crate) fn write_local_api_failure(
    data_dir: &Path,
    marker_path: &Path,
    provider: &str,
    launch_id: &str,
) -> Result<(), ProviderHookHealthMarkerError> {
    if !matches!(provider, "claude" | "codex") || launch_id.trim().is_empty() {
        return Err(ProviderHookHealthMarkerError::InvalidPath);
    }
    validate_marker_path(data_dir, marker_path)?;
    let parent = marker_path
        .parent()
        .ok_or(ProviderHookHealthMarkerError::InvalidPath)?;
    std::fs::create_dir_all(parent).map_err(|_| ProviderHookHealthMarkerError::Unavailable)?;
    let contents = serde_json::to_vec(&serde_json::json!({
        "provider": provider,
        "launchId": launch_id,
        "reason": "local_api_unavailable",
    }))
    .map_err(|_| ProviderHookHealthMarkerError::Unavailable)?;
    std::fs::write(marker_path, contents).map_err(|_| ProviderHookHealthMarkerError::Unavailable)
}

pub(crate) fn clear_local_api_failure(
    data_dir: &Path,
    marker_path: &Path,
) -> Result<(), ProviderHookHealthMarkerError> {
    validate_marker_path(data_dir, marker_path)?;
    match std::fs::remove_file(marker_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(ProviderHookHealthMarkerError::Unavailable),
    }
}

fn validate_marker_path(
    data_dir: &Path,
    marker_path: &Path,
) -> Result<(), ProviderHookHealthMarkerError> {
    let root = data_dir.join("provider-launches");
    let relative = marker_path
        .strip_prefix(&root)
        .map_err(|_| ProviderHookHealthMarkerError::InvalidPath)?;
    if marker_path.file_name().and_then(|value| value.to_str()) != Some("hook-health.json")
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ProviderHookHealthMarkerError::InvalidPath);
    }
    Ok(())
}

pub(crate) fn read_local_api_failures(
    data_dir: &Path,
    limit: usize,
) -> Result<Vec<RawProviderHookHealthFailure>, ProviderHookHealthMarkerError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let root = data_dir.join("provider-launches");
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut session_directories = directories(&root)?;
    session_directories.sort();
    let mut marker_paths = Vec::new();
    for session_directory in session_directories {
        let mut launch_directories = directories(&session_directory)?;
        launch_directories.sort();
        for launch_directory in launch_directories {
            let marker_path = launch_directory.join("hook-health.json");
            let Ok(metadata) = std::fs::symlink_metadata(&marker_path) else {
                continue;
            };
            if !metadata.file_type().is_file() || metadata.len() > MAX_MARKER_BYTES {
                continue;
            }
            marker_paths.push(marker_path);
            if marker_paths.len() == limit {
                break;
            }
        }
        if marker_paths.len() == limit {
            break;
        }
    }
    marker_paths
        .into_iter()
        .map(|path| {
            std::fs::read(path)
                .map(|contents| RawProviderHookHealthFailure { contents })
                .map_err(|_| ProviderHookHealthMarkerError::Unavailable)
        })
        .collect()
}

fn directories(root: &Path) -> Result<Vec<std::path::PathBuf>, ProviderHookHealthMarkerError> {
    std::fs::read_dir(root)
        .map_err(|_| ProviderHookHealthMarkerError::Unavailable)?
        .filter_map(|entry| match entry {
            Ok(entry) => match entry.file_type() {
                Ok(file_type) if file_type.is_dir() && !file_type.is_symlink() => {
                    Some(Ok(entry.path()))
                }
                Ok(_) => None,
                Err(_) => Some(Err(ProviderHookHealthMarkerError::Unavailable)),
            },
            Err(_) => Some(Err(ProviderHookHealthMarkerError::Unavailable)),
        })
        .collect()
}
