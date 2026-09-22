use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::domain::repository::worktree_operation::{
    WorktreeOperationLease, WorktreeOperationLocks,
};
use crate::domain::repository::{normalize_repo_path, RepositoryError};

#[derive(Clone)]
pub(crate) struct FileWorktreeOperationLocks {
    directory: PathBuf,
}

struct FileLease {
    files: Vec<File>,
    locks: FileWorktreeOperationLocks,
    key: String,
}
impl WorktreeOperationLease for FileLease {}

impl FileWorktreeOperationLocks {
    pub(crate) fn new(app_data_dir: &Path) -> Self {
        Self {
            directory: app_data_dir.join("worktree-operations"),
        }
    }

    fn open(&self, name: &str) -> Result<File, RepositoryError> {
        Ok(OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.directory.join(name))?)
    }

    fn registry_lock(&self) -> Result<File, RepositoryError> {
        std::fs::create_dir_all(&self.directory)?;
        let registry = self.open("registry")?;
        fs2::FileExt::lock_exclusive(&registry)?;
        Ok(registry)
    }

    fn lease(&self, identity: &str, deleting: bool) -> Result<FileLease, RepositoryError> {
        let identity = worktree_identity(identity)?;
        let key = hex::encode(Sha256::digest(identity.to_string_lossy().as_bytes()));
        let registry = self.registry_lock()?;
        let mut lease = FileLease {
            files: Vec::new(),
            locks: self.clone(),
            key,
        };
        let result = (|| {
            lease
                .files
                .push(self.open(&format!("{}.admission", lease.key))?);
            let admission = &lease.files[0];
            if deleting {
                fs2::FileExt::try_lock_exclusive(admission)
            } else {
                fs2::FileExt::try_lock_shared(admission)
            }
            .map_err(admission_error)?;
            lease
                .files
                .push(self.open(&format!("{}.active", lease.key))?);
            if !deleting {
                fs2::FileExt::try_lock_shared(&lease.files[1]).map_err(admission_error)?;
                fs2::FileExt::unlock(&lease.files[0])?;
            }
            Ok::<_, RepositoryError>(())
        })();
        drop(registry);
        result?;
        Ok(lease)
    }

    fn remove_idle(&self, key: &str) -> Result<(), RepositoryError> {
        let _registry = self.registry_lock()?;
        let mut files = Vec::new();
        for suffix in ["admission", "active"] {
            let path = self.directory.join(format!("{key}.{suffix}"));
            let file = match OpenOptions::new().read(true).write(true).open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => files.push((file, path)),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(error.into()),
            }
        }
        for (_, path) in &files {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

impl Drop for FileLease {
    fn drop(&mut self) {
        self.files.clear();
        if let Err(error) = self.locks.remove_idle(&self.key) {
            log::warn!("worktree operation lock cleanup failed: {error}");
        }
    }
}

pub(crate) fn worktree_identity(identity: &str) -> Result<PathBuf, RepositoryError> {
    let path = PathBuf::from(normalize_repo_path(identity));
    for ancestor in path.ancestors() {
        match ancestor.canonicalize() {
            Ok(canonical) => {
                let suffix = path.strip_prefix(ancestor).expect("path ancestor");
                return Ok(if suffix.as_os_str().is_empty() {
                    canonical
                } else {
                    canonical.join(suffix)
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(path)
}

fn admission_error(error: std::io::Error) -> RepositoryError {
    if error.kind() == std::io::ErrorKind::WouldBlock {
        RepositoryError::rule("worktree deletion is in progress")
    } else {
        error.into()
    }
}

#[async_trait::async_trait]
impl WorktreeOperationLocks for FileWorktreeOperationLocks {
    fn mutation(&self, identity: &str) -> Result<Box<dyn WorktreeOperationLease>, RepositoryError> {
        Ok(Box::new(self.lease(identity, false)?))
    }

    async fn deletion(
        &self,
        identity: &str,
    ) -> Result<Box<dyn WorktreeOperationLease>, RepositoryError> {
        let lease = self.lease(identity, true)?;
        loop {
            match fs2::FileExt::try_lock_exclusive(&lease.files[1]) {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(Box::new(lease))
    }
}

#[cfg(test)]
#[path = "worktree_operation_test.rs"]
mod worktree_operation_tests;
