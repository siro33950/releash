use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

const LOCAL_API_DISCOVERY_FILE_NAME: &str = "local-api.json";

pub(crate) fn local_api_discovery_path(data_dir: &Path) -> PathBuf {
    data_dir.join(LOCAL_API_DISCOVERY_FILE_NAME)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct LocalApiDiscovery {
    pub(crate) port: u16,
    pub(crate) token: String,
    pub(crate) pid: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct LocalApiDiscoveryFile {
    path: PathBuf,
    discovery: LocalApiDiscovery,
}

impl LocalApiDiscoveryFile {
    pub(crate) fn create(data_dir: &Path, discovery: LocalApiDiscovery) -> io::Result<Self> {
        fs::create_dir_all(data_dir)?;
        let path = local_api_discovery_path(data_dir);
        let temporary_path = data_dir.join(format!(
            ".{LOCAL_API_DISCOVERY_FILE_NAME}.{}.tmp",
            uuid::Uuid::new_v4()
        ));
        let encoded = serde_json::to_vec(&discovery).map_err(io::Error::other)?;

        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options.open(&temporary_path)?;
            file.write_all(&encoded)?;
            file.sync_all()?;
            #[cfg(unix)]
            fs::set_permissions(&temporary_path, fs::Permissions::from_mode(0o600))?;

            // A previous process may have left stale discovery behind. The temporary
            // file is already complete and private before replacing it.
            fs::rename(&temporary_path, &path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result?;

        Ok(Self { path, discovery })
    }

    pub(crate) fn remove_if_owned(&self) -> io::Result<()> {
        let current = match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice::<LocalApiDiscovery>(&bytes).ok(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        if current
            .as_ref()
            .is_some_and(|value| value != &self.discovery)
        {
            return Ok(());
        }
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_file_is_private_and_removed_by_its_owner() {
        let directory = tempfile::tempdir().unwrap();
        let discovery = LocalApiDiscovery {
            port: 43123,
            token: "secret-token".to_string(),
            pid: 42,
        };
        let file = LocalApiDiscoveryFile::create(directory.path(), discovery.clone()).unwrap();

        let decoded: LocalApiDiscovery =
            serde_json::from_slice(&fs::read(file.path()).unwrap()).unwrap();
        assert_eq!(decoded, discovery);
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(file.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );

        file.remove_if_owned().unwrap();
        assert!(!file.path().exists());
    }

    #[test]
    fn stale_owner_does_not_remove_newer_discovery() {
        let directory = tempfile::tempdir().unwrap();
        let stale = LocalApiDiscoveryFile::create(
            directory.path(),
            LocalApiDiscovery {
                port: 40001,
                token: "stale".to_string(),
                pid: 1,
            },
        )
        .unwrap();
        let current = LocalApiDiscoveryFile::create(
            directory.path(),
            LocalApiDiscovery {
                port: 40002,
                token: "current".to_string(),
                pid: 2,
            },
        )
        .unwrap();

        stale.remove_if_owned().unwrap();
        assert!(current.path().exists());
        current.remove_if_owned().unwrap();
    }
}
