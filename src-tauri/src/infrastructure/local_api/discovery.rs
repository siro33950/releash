use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub use releash_client::discovery::{
    lookup_process_start_time, process_start_time, LocalApiDiscovery, ProcessStartTimeLookup,
};

#[derive(Debug, Clone)]
pub struct LocalApiDiscoveryFile {
    path: PathBuf,
    discovery: LocalApiDiscovery,
}

impl LocalApiDiscoveryFile {
    #[cfg(any(test, feature = "test-support"))]
    pub fn create_client(data_dir: &Path, discovery: LocalApiDiscovery) -> io::Result<Self> {
        Self::create_named(data_dir, "client-api.json", discovery)
    }

    #[cfg(any(test, feature = "test-support"))]
    fn create_named(data_dir: &Path, name: &str, discovery: LocalApiDiscovery) -> io::Result<Self> {
        let file = Self::prepare_named(data_dir, name, discovery);
        file.publish()?;
        Ok(file)
    }

    pub(crate) fn prepare_named(data_dir: &Path, name: &str, discovery: LocalApiDiscovery) -> Self {
        Self {
            path: data_dir.join(name),
            discovery,
        }
    }

    pub(crate) fn publish(&self) -> io::Result<()> {
        let data_dir = self.path.parent().expect("discovery directory");
        fs::create_dir_all(data_dir)?;
        let path = &self.path;
        let name = path
            .file_name()
            .expect("discovery file name")
            .to_string_lossy();
        let discovery = &self.discovery;
        let temporary_path = data_dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
        let encoded = serde_json::to_vec(discovery).map_err(io::Error::other)?;

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
            fs::rename(&temporary_path, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result?;

        Ok(())
    }

    pub fn remove_if_owned(&self) -> io::Result<()> {
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn path(&self) -> &Path {
        &self.path
    }
}
