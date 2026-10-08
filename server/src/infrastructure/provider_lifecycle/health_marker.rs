use std::path::Path;

const MAX_MARKER_BYTES: u64 = 4 * 1024;

pub(crate) struct RawProviderHookHealthFailure {
    pub(crate) contents: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderHookHealthMarkerError {
    #[error("Provider Hook health marker path is invalid")]
    InvalidPath,
    #[error("Provider Hook health marker is unavailable")]
    Io(#[from] std::io::Error),
}

pub(crate) fn read_local_api_failures(
    data_dir: &Path,
    limit: usize,
) -> Result<
    Vec<Result<RawProviderHookHealthFailure, ProviderHookHealthMarkerError>>,
    ProviderHookHealthMarkerError,
> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let root = data_dir.join("provider-launches");
    if !root
        .try_exists()
        .map_err(ProviderHookHealthMarkerError::Io)?
    {
        return Ok(Vec::new());
    }
    let mut session_directories = directories(&root)?;
    session_directories.sort();
    let records = session_directories
        .into_iter()
        .flat_map(|session_directory| match directories(&session_directory) {
            Err(error) => vec![Err(error)].into_iter(),
            Ok(mut launch_directories) => {
                launch_directories.sort();
                launch_directories
                    .into_iter()
                    .map(Ok)
                    .collect::<Vec<_>>()
                    .into_iter()
            }
        })
        .filter_map(|directory| match directory {
            Err(error) => Some(Err(error)),
            Ok(directory) => read_marker(&directory.join("hook-health.json")),
        })
        .take(limit)
        .collect();
    Ok(records)
}

fn read_marker(
    marker_path: &Path,
) -> Option<Result<RawProviderHookHealthFailure, ProviderHookHealthMarkerError>> {
    let metadata = match std::fs::symlink_metadata(marker_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => return Some(Err(ProviderHookHealthMarkerError::Io(error))),
    };
    Some(
        if !metadata.file_type().is_file() || metadata.len() > MAX_MARKER_BYTES {
            Err(ProviderHookHealthMarkerError::InvalidPath)
        } else {
            std::fs::read(marker_path)
                .map(|contents| RawProviderHookHealthFailure { contents })
                .map_err(ProviderHookHealthMarkerError::Io)
        },
    )
}

fn directories(root: &Path) -> Result<Vec<std::path::PathBuf>, ProviderHookHealthMarkerError> {
    std::fs::read_dir(root)
        .map_err(ProviderHookHealthMarkerError::Io)?
        .filter_map(|entry| match entry {
            Ok(entry) => match entry.file_type() {
                Ok(file_type) if file_type.is_dir() && !file_type.is_symlink() => {
                    Some(Ok(entry.path()))
                }
                Ok(_) => None,
                Err(error) => Some(Err(ProviderHookHealthMarkerError::Io(error))),
            },
            Err(error) => Some(Err(ProviderHookHealthMarkerError::Io(error))),
        })
        .collect()
}
