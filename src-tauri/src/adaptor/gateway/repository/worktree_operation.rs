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
        wait_lock(&registry)?;
        Ok(registry)
    }

    fn lease(&self, identity: &str, deleting: bool) -> Result<FileLease, RepositoryError> {
        let registry = self.registry_lock()?;
        self.lease_registered(identity, deleting, registry)
    }

    fn lease_registered(
        &self,
        identity: &str,
        deleting: bool,
        registry: File,
    ) -> Result<FileLease, RepositoryError> {
        let identity = worktree_identity(identity)?;
        let key = hex::encode(Sha256::digest(identity.to_string_lossy().as_bytes()));
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
        let registry = self.open("registry")?;
        match fs2::FileExt::try_lock_exclusive(&registry) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let locks = self.clone();
                let key = key.to_string();
                // ponytail: 競合ごとに1スレッド。競合が常態化したら単一cleanup workerへ集約する。
                std::thread::Builder::new()
                    .name("worktree-lock-cleanup".into())
                    .spawn(move || {
                        let result = crate::adaptor::gateway::shared::file_lock::exclusive(
                            &registry,
                            &crate::domain::operation_context::OperationContext::default(),
                        )
                        .map_err(lock_error)
                        .and_then(|()| locks.remove_idle_registered(&key, registry));
                        if let Err(error) = result {
                            log::warn!("worktree operation lock cleanup failed: {error}");
                        }
                    })?;
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        }
        self.remove_idle_registered(key, registry)
    }

    fn remove_idle_registered(&self, key: &str, _registry: File) -> Result<(), RepositoryError> {
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
        for file in &self.files {
            if let Err(error) = fs2::FileExt::unlock(file) {
                log::warn!("worktree operation unlock failed: {error}");
            }
        }
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
        let context = crate::other::operation_context::current();
        std::fs::create_dir_all(&self.directory)?;
        let registry = self.open("registry")?;
        crate::adaptor::gateway::shared::file_lock::exclusive_async(&registry, &context)
            .await
            .map_err(lock_error)?;
        let lease = self.lease_registered(identity, true, registry)?;
        crate::adaptor::gateway::shared::file_lock::exclusive_async(&lease.files[1], &context)
            .await
            .map_err(lock_error)?;
        Ok(Box::new(lease))
    }
}

#[cfg(test)]
#[path = "worktree_operation_test.rs"]
mod worktree_operation_tests;

fn wait_lock(file: &File) -> Result<(), RepositoryError> {
    crate::adaptor::gateway::shared::file_lock::exclusive(
        file,
        &crate::other::operation_context::current(),
    )
    .map_err(lock_error)
}
fn lock_error(error: crate::adaptor::gateway::shared::file_lock::LockError) -> RepositoryError {
    match error {
        crate::adaptor::gateway::shared::file_lock::LockError::Io(error) => error.into(),
        crate::adaptor::gateway::shared::file_lock::LockError::Stopped(error) => error.into(),
    }
}
