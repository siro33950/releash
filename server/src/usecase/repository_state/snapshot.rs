use serde::{Deserialize, Serialize};

use crate::usecase::repository_dto::FileStatusDto;

#[derive(Debug, Clone)]
pub struct RepositorySnapshot {
    pub version: u64,
    pub status: Vec<FileStatusDto>,
    /// 未コミットの変更の数。
    pub dirty_count: usize,
}

impl RepositorySnapshot {
    pub fn loading() -> Self {
        Self {
            version: 0,
            status: Vec::new(),
            dirty_count: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositorySnapshotParts {
    pub status: Vec<FileStatusDto>,
    pub dirty_count: usize,
}

impl RepositorySnapshotParts {
    pub fn into_snapshot(self, version: u64) -> RepositorySnapshot {
        RepositorySnapshot {
            version,
            status: self.status,
            dirty_count: self.dirty_count,
        }
    }
}
