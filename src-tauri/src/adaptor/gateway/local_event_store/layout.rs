//! Fixed production layout and initial-create evidence for the local event store.
//!
//! The layout deliberately derives exactly three application-owned paths from
//! app-data. It never enumerates app-data and has no pointer, generation, or
//! legacy-source concept.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::adaptor::gateway::local_event_store::connection::set_owner_only_permissions;
use crate::adaptor::gateway::local_event_store::fault::{FaultInjector, InitialCreateFaultPoint};
pub use crate::infrastructure::app_data_path::{
    AppDataPathObserver as StorePathObserver, AppDataPathOperation as StorePathOperation,
    NoopAppDataPathObserver as NoopStorePathObserver,
};

pub const DATABASE_FILE: &str = "local-event-store.sqlite3";
pub const WRITER_LOCK_FILE: &str = "local-event-store.lock";
pub const INITIAL_CREATE_EVIDENCE_FILE: &str = "local-event-store.initial-create";

const EVIDENCE_MAGIC: &str = "releash-local-event-store-initial-create";
const EVIDENCE_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct StoreLayout {
    app_data_root: PathBuf,
    observer: Arc<dyn StorePathObserver>,
}

impl std::fmt::Debug for dyn StorePathObserver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("StorePathObserver")
    }
}

impl StoreLayout {
    pub fn new(app_data_root: &Path) -> Self {
        Self::with_observer(app_data_root, Arc::new(NoopStorePathObserver))
    }

    pub fn with_observer(app_data_root: &Path, observer: Arc<dyn StorePathObserver>) -> Self {
        Self {
            app_data_root: app_data_root.to_path_buf(),
            observer,
        }
    }

    pub fn database_path(&self) -> PathBuf {
        self.app_data_root.join(DATABASE_FILE)
    }

    pub fn writer_lock_path(&self) -> PathBuf {
        self.app_data_root.join(WRITER_LOCK_FILE)
    }

    pub fn initial_create_evidence_path(&self) -> PathBuf {
        self.app_data_root.join(INITIAL_CREATE_EVIDENCE_FILE)
    }

    pub fn ensure_app_data_root(&self) -> std::io::Result<()> {
        self.observe(StorePathOperation::Write, &self.app_data_root);
        std::fs::create_dir_all(&self.app_data_root)?;
        self.observe(StorePathOperation::Metadata, &self.app_data_root);
        set_owner_only_permissions(&self.app_data_root)
    }

    pub fn sync_app_data_root(&self) -> std::io::Result<()> {
        self.observe(StorePathOperation::Open, &self.app_data_root);
        self.observe(StorePathOperation::Sync, &self.app_data_root);
        sync_directory(&self.app_data_root)
    }

    pub fn observe(&self, operation: StorePathOperation, path: &Path) {
        self.observer.observe(operation, path);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitialCreateEvidenceState {
    Absent,
    Valid,
    Invalid,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InitialCreateEvidence {
    magic: String,
    version: u32,
    checksum_sha256: String,
}

fn evidence_checksum() -> String {
    hex::encode(Sha256::digest(format!(
        "{EVIDENCE_MAGIC}\0{EVIDENCE_VERSION}"
    )))
}

pub fn inspect_initial_create_evidence(
    layout: &StoreLayout,
) -> Result<InitialCreateEvidenceState, std::io::Error> {
    let path = layout.initial_create_evidence_path();
    layout.observe(StorePathOperation::Read, &path);
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(InitialCreateEvidenceState::Absent);
        }
        Err(error) => return Err(error),
    };
    let Ok(evidence) = serde_json::from_slice::<InitialCreateEvidence>(&bytes) else {
        return Ok(InitialCreateEvidenceState::Invalid);
    };
    if evidence.magic != EVIDENCE_MAGIC
        || evidence.version != EVIDENCE_VERSION
        || evidence.checksum_sha256 != evidence_checksum()
    {
        return Ok(InitialCreateEvidenceState::Invalid);
    }
    Ok(InitialCreateEvidenceState::Valid)
}

pub fn create_initial_create_evidence_with_fault(
    layout: &StoreLayout,
    fault: Option<&FaultInjector>,
) -> Result<(), std::io::Error> {
    if fault.is_some_and(|fault| {
        fault.take_initial_create_fault(InitialCreateFaultPoint::BeforeEvidenceCreate)
    }) {
        #[cfg(test)]
        fault
            .expect("armed initial-create fault injector")
            .crash_initial_create_process_if_armed(InitialCreateFaultPoint::BeforeEvidenceCreate);
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "injected stop before initial-create evidence",
        ));
    }
    let evidence = InitialCreateEvidence {
        magic: EVIDENCE_MAGIC.to_string(),
        version: EVIDENCE_VERSION,
        checksum_sha256: evidence_checksum(),
    };
    let encoded = serde_json::to_vec(&evidence).map_err(std::io::Error::other)?;
    let path = layout.initial_create_evidence_path();
    layout.observe(StorePathOperation::Write, &path);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    if fault.is_some_and(|fault| {
        fault.take_initial_create_fault(InitialCreateFaultPoint::AfterPartialEvidenceWrite)
    }) {
        file.write_all(&encoded[..encoded.len() / 2])?;
        #[cfg(test)]
        fault
            .expect("armed initial-create fault injector")
            .crash_initial_create_process_if_armed(
                InitialCreateFaultPoint::AfterPartialEvidenceWrite,
            );
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "injected stop after partial initial-create evidence",
        ));
    }
    file.write_all(&encoded)?;
    layout.observe(StorePathOperation::Sync, &path);
    file.sync_all()?;
    if fault.is_some_and(|fault| {
        fault.take_initial_create_fault(InitialCreateFaultPoint::AfterEvidenceFileSync)
    }) {
        #[cfg(test)]
        fault
            .expect("armed initial-create fault injector")
            .crash_initial_create_process_if_armed(InitialCreateFaultPoint::AfterEvidenceFileSync);
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "injected stop after initial-create evidence sync",
        ));
    }
    layout.observe(StorePathOperation::Metadata, &path);
    set_owner_only_permissions(&path)?;
    layout.sync_app_data_root()?;
    if fault.is_some_and(|fault| {
        fault.take_initial_create_fault(InitialCreateFaultPoint::AfterEvidenceDirectorySync)
    }) {
        #[cfg(test)]
        fault
            .expect("armed initial-create fault injector")
            .crash_initial_create_process_if_armed(
                InitialCreateFaultPoint::AfterEvidenceDirectorySync,
            );
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "injected stop after initial-create directory sync",
        ));
    }
    Ok(())
}

pub fn remove_initial_create_evidence(layout: &StoreLayout) -> Result<(), std::io::Error> {
    let path = layout.initial_create_evidence_path();
    layout.observe(StorePathOperation::Remove, &path);
    match std::fs::remove_file(path) {
        Ok(()) => layout.sync_app_data_root(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn replace_invalid_evidence_for_absent_database_with_fault(
    layout: &StoreLayout,
    fault: Option<&FaultInjector>,
) -> Result<(), std::io::Error> {
    let path = layout.initial_create_evidence_path();
    layout.observe(StorePathOperation::Remove, &path);
    match std::fs::remove_file(path) {
        Ok(()) => layout.sync_app_data_root()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    create_initial_create_evidence_with_fault(layout, fault)
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
