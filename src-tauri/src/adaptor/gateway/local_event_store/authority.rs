//! `local-event-store/authority-v1.json`: the only cutover authority that
//! lives outside SQLite.
//!
//! The pointer is a closed union with a versioned envelope and checksum,
//! written with temp-file write, file sync, rename, and parent directory
//! sync as a compare-and-swap. It locates migration staging and cutover
//! only; normal batch commits never consult it.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::adaptor::gateway::local_event_store::connection::set_owner_only_permissions;

pub const STORE_DIRECTORY: &str = "local-event-store";
pub const AUTHORITY_FILE: &str = "authority-v1.json";
pub const GENERATIONS_DIRECTORY: &str = "generations";
const ENVELOPE_VERSION: i64 = 1;

/// In-flight legacy migration locator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityMigrationRef {
    pub migration_id: String,
    pub staging_generation_id: String,
}

/// Closed authority union from the issues-1499 design "Physical layout".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LocalStoreAuthorityPointerV1 {
    Legacy {
        source_generation_id: String,
        migration: Option<AuthorityMigrationRef>,
    },
    Sqlite {
        generation_id: String,
        store_id: String,
        activated_migration_id: Option<String>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthorityEnvelope {
    version: i64,
    checksum_sha256: String,
    authority: LocalStoreAuthorityPointerV1,
}

#[derive(Debug)]
pub enum AuthorityError {
    /// The pointer on disk did not match the expected value of the CAS.
    CasConflict {
        current: Option<LocalStoreAuthorityPointerV1>,
    },
    Corrupt {
        reason: String,
    },
    Io(std::io::Error),
}

impl fmt::Display for AuthorityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CasConflict { current } => write!(
                f,
                "authority pointer CAS conflict (current_present={})",
                current.is_some()
            ),
            Self::Corrupt { reason } => write!(f, "authority pointer corrupt: {reason}"),
            Self::Io(inner) => write!(f, "authority pointer io error: {inner}"),
        }
    }
}

impl std::error::Error for AuthorityError {}

impl From<std::io::Error> for AuthorityError {
    fn from(inner: std::io::Error) -> Self {
        Self::Io(inner)
    }
}

/// Fault points for authority CAS tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityFaultPoint {
    TempWritten,
    TempSynced,
    AuthorityRenamed,
}

/// Filesystem layout of the store inside app-data.
#[derive(Debug, Clone)]
pub struct StoreLayout {
    root: PathBuf,
}

impl StoreLayout {
    pub fn new(app_data_root: &Path) -> Self {
        Self {
            root: app_data_root.join(STORE_DIRECTORY),
        }
    }

    pub fn store_directory(&self) -> &Path {
        &self.root
    }

    pub fn authority_path(&self) -> PathBuf {
        self.root.join(AUTHORITY_FILE)
    }

    pub fn generations_directory(&self) -> PathBuf {
        self.root.join(GENERATIONS_DIRECTORY)
    }

    pub fn generation_database_path(&self, generation_id: &str) -> PathBuf {
        self.generations_directory()
            .join(format!("{generation_id}.sqlite3"))
    }

    /// Create the owner-only directory tree.
    pub fn ensure_directories(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.generations_directory())?;
        set_owner_only_permissions(&self.root)?;
        set_owner_only_permissions(&self.generations_directory())?;
        Ok(())
    }
}

fn checksum_of(authority: &LocalStoreAuthorityPointerV1) -> Result<String, AuthorityError> {
    let canonical = serde_json::to_vec(authority).map_err(|error| AuthorityError::Corrupt {
        reason: format!("authority serialization failed: {error}"),
    })?;
    Ok(hex::encode(Sha256::digest(canonical)))
}

/// Read and verify the current pointer. `Ok(None)` when the file is absent.
pub fn read_authority(
    layout: &StoreLayout,
) -> Result<Option<LocalStoreAuthorityPointerV1>, AuthorityError> {
    let path = layout.authority_path();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let envelope: AuthorityEnvelope =
        serde_json::from_slice(&bytes).map_err(|error| AuthorityError::Corrupt {
            reason: format!("authority envelope parse failed: {error}"),
        })?;
    if envelope.version != ENVELOPE_VERSION {
        return Err(AuthorityError::Corrupt {
            reason: format!(
                "unsupported authority envelope version {}",
                envelope.version
            ),
        });
    }
    let expected = checksum_of(&envelope.authority)?;
    if envelope.checksum_sha256 != expected {
        return Err(AuthorityError::Corrupt {
            reason: "authority checksum mismatch".to_string(),
        });
    }
    Ok(Some(envelope.authority))
}

fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Compare-and-swap the pointer: fails with `CasConflict` unless the current
/// on-disk pointer equals `expected`. Fault injection interrupts the write at
/// the requested point to simulate a crash; a fresh `read_authority` then
/// decides which side of the rename the crash landed on.
pub fn cas_authority(
    layout: &StoreLayout,
    expected: Option<&LocalStoreAuthorityPointerV1>,
    new: &LocalStoreAuthorityPointerV1,
    fault: Option<AuthorityFaultPoint>,
) -> Result<(), AuthorityError> {
    layout.ensure_directories()?;
    let current = read_authority(layout)?;
    if current.as_ref() != expected {
        return Err(AuthorityError::CasConflict { current });
    }

    let envelope = AuthorityEnvelope {
        version: ENVELOPE_VERSION,
        checksum_sha256: checksum_of(new)?,
        authority: new.clone(),
    };
    let encoded =
        serde_json::to_vec_pretty(&envelope).map_err(|error| AuthorityError::Corrupt {
            reason: format!("authority envelope serialization failed: {error}"),
        })?;

    let temp_path = layout.authority_path().with_extension("json.tmp");
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)?;
        file.write_all(&encoded)?;
        if fault == Some(AuthorityFaultPoint::TempWritten) {
            return Err(AuthorityError::Io(std::io::Error::other(
                "injected fault after temp write",
            )));
        }
        file.sync_all()?;
    }
    set_owner_only_permissions(&temp_path)?;
    if fault == Some(AuthorityFaultPoint::TempSynced) {
        return Err(AuthorityError::Io(std::io::Error::other(
            "injected fault after temp sync",
        )));
    }
    std::fs::rename(&temp_path, layout.authority_path())?;
    if fault == Some(AuthorityFaultPoint::AuthorityRenamed) {
        return Err(AuthorityError::Io(std::io::Error::other(
            "injected fault after rename (before directory sync)",
        )));
    }
    sync_directory(layout.store_directory())?;
    Ok(())
}
