use crate::domain::client_operation::handoff::{
    validate_reference, ClientHandoffReference, ClientHandoffRepository,
};
use crate::usecase::client_handoff_query::{ClientHandoffQueryService, ClientHandoffSummary};
use std::io::Write;
use std::path::PathBuf;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredReference {
    id: String,
    command: String,
    fingerprint: Vec<u8>,
    ordering_target: Vec<u8>,
}

pub(crate) struct ClientHandoffFiles {
    directory: PathBuf,
    writes: parking_lot::Mutex<()>,
}
impl ClientHandoffFiles {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            writes: parking_lot::Mutex::new(()),
        }
    }
}
impl ClientHandoffRepository for ClientHandoffFiles {
    fn remember(&self, reference: &ClientHandoffReference) -> Result<(), String> {
        let _guard = self.writes.lock();
        std::fs::create_dir_all(&self.directory).map_err(|e| e.to_string())?;
        if !self
            .directory
            .join(format!("{}.json", reference.id))
            .exists()
        {
            let mut count = 0;
            for entry in std::fs::read_dir(&self.directory).map_err(|e| e.to_string())? {
                if entry
                    .map_err(|e| e.to_string())?
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json")
                {
                    count += 1;
                }
                if count >= crate::domain::client_operation::policy::MAX_UNACKNOWLEDGED_OPERATIONS {
                    return Err("Too many unresolved desktop operations.".into());
                }
            }
        }
        let mut file =
            tempfile::NamedTempFile::new_in(&self.directory).map_err(|e| e.to_string())?;
        serde_json::to_writer(
            &mut file,
            &StoredReference {
                id: reference.id.clone(),
                command: reference.command.clone(),
                fingerprint: reference.fingerprint.clone(),
                ordering_target: reference.ordering_target.clone(),
            },
        )
        .map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(self.directory.join(format!("{}.json", reference.id)))
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    fn forget(&self, id: &str) -> Result<(), String> {
        let _guard = self.writes.lock();
        match std::fs::remove_file(self.directory.join(format!("{id}.json"))) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}
impl ClientHandoffQueryService for ClientHandoffFiles {
    fn list(&self) -> Result<Vec<ClientHandoffSummary>, String> {
        let _guard = self.writes.lock();
        let entries = match std::fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.to_string()),
        };
        let mut references = Vec::new();
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            if references.len()
                >= crate::domain::client_operation::policy::MAX_UNACKNOWLEDGED_OPERATIONS
            {
                return Err("Too many unresolved desktop operations.".into());
            }
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let stored: StoredReference = serde_json::from_reader(std::io::Read::take(file, 4096))
                .map_err(|e| e.to_string())?;
            validate_reference(
                &stored.id,
                &stored.command,
                &stored.fingerprint,
                &stored.ordering_target,
            )?;
            references.push(ClientHandoffSummary {
                id: stored.id,
                command: stored.command,
                fingerprint: stored.fingerprint,
                ordering_target: stored.ordering_target,
            });
        }
        Ok(references)
    }
}

#[cfg(test)]
#[path = "client_handoff_test.rs"]
mod client_handoff_tests;
