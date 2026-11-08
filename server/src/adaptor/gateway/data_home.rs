use crate::domain::daemon::data_home::{
    DataEntry, DataHome, DataHomeStore, EntryShape, LegacyServer, LegacyServerRecord, LegacySurvey,
    MoveAction, ProcessLookup, RELOCATION_MARKER,
};
use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
use crate::infrastructure::local_api::{lookup_process_start_time, LocalApiDiscovery};
use std::fs;
use std::io;
use std::path::Path;

/// ファイルシステム上の保存先を観測し、項目を `rename` で移す `DataHomeStore`。
/// 旧保存先のサーバの状態は、旧保存先の `client-api.json` と process の開始時刻で判定する。
pub(crate) struct LocalDataHomeStore;

impl DataHomeStore for LocalDataHomeStore {
    fn survey_legacy(&self, home: &DataHome) -> Result<LegacySurvey, TechnicalFailure> {
        let legacy = home.legacy.as_deref();
        Ok(LegacySurvey {
            entries: legacy
                .map(entries)
                .transpose()
                .map_err(failure)?
                .unwrap_or_default(),
            server: legacy.map(server).unwrap_or(LegacyServer::Absent),
            relocation_started: legacy.is_some_and(|path| path.join(RELOCATION_MARKER).exists()),
        })
    }

    fn current_entries(&self, home: &DataHome) -> Result<Vec<DataEntry>, TechnicalFailure> {
        entries(&home.current).map_err(failure)
    }

    fn relocate(&self, home: &DataHome, entries: &[String]) -> Result<(), TechnicalFailure> {
        let Some(legacy) = &home.legacy else {
            return Ok(());
        };
        if !legacy.exists() {
            return Ok(());
        }
        let marker = legacy.join(RELOCATION_MARKER);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
        {
            Ok(file) => file.sync_all().map_err(failure)?,
            Err(error)
                if error.kind() == io::ErrorKind::AlreadyExists
                    && marker.symlink_metadata().map_err(failure)?.is_file() => {}
            Err(error) => return Err(failure(error)),
        }
        fs::create_dir_all(&home.current).map_err(failure)?;
        crate::infrastructure::local_event_store_connection::set_owner_only_permissions(
            &home.current,
        )
        .map_err(failure)?;
        for name in entries {
            if Path::new(name).file_name().and_then(|value| value.to_str()) != Some(name.as_str()) {
                return Err(failure(io::Error::other("invalid relocation entry")));
            }
            move_entry(&legacy.join(name), &home.current.join(name)).map_err(failure)?;
        }
        match fs::remove_file(legacy.join(RELOCATION_MARKER)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(failure(error)),
        }
        match fs::remove_dir(legacy) {
            Ok(()) => Ok(()),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
                ) =>
            {
                Ok(())
            }
            Err(error) => Err(failure(error)),
        }
    }
}

fn failure(error: io::Error) -> TechnicalFailure {
    TechnicalFailure {
        nature: TechnicalFailureNature::Other,
        message: error.to_string(),
    }
}

fn entries(path: &Path) -> io::Result<Vec<DataEntry>> {
    let directory = match fs::read_dir(path) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    directory
        .map(|entry| {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| io::Error::other("non-UTF-8 relocation entry"))?;
            Ok(DataEntry {
                name,
                bytes: bytes(&entry.path())?,
            })
        })
        .collect()
}

fn bytes(path: &Path) -> io::Result<u64> {
    let metadata = path.symlink_metadata()?;
    if !metadata.is_dir() {
        return Ok(metadata.len());
    }
    fs::read_dir(path)?.try_fold(0u64, |total, entry| {
        total
            .checked_add(bytes(&entry?.path())?)
            .ok_or_else(|| io::Error::other("directory size overflow"))
    })
}

fn server(path: &Path) -> LegacyServer {
    LegacyServer::judge(record(path), |pid| {
        let process = lookup_process_start_time(pid);
        match (process.process_list_available, process.start_time) {
            (false, _) => ProcessLookup::Unavailable,
            (true, None) => ProcessLookup::NotFound,
            (true, Some(start)) => ProcessLookup::StartedAt(start),
        }
    })
}

fn record(path: &Path) -> LegacyServerRecord {
    let bytes = match fs::read(path.join("client-api.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return LegacyServerRecord::Missing
        }
        Err(_) => return LegacyServerRecord::Unreadable,
    };
    let descriptor = crate::infrastructure::client_protocol::descriptor::pool()
        .get_message_by_name("releash.client.v1.LocalApiDiscovery")
        .expect("discovery descriptor");
    let mut deserializer = serde_json::Deserializer::from_slice(&bytes);
    let discovery = prost_reflect::DynamicMessage::deserialize(descriptor, &mut deserializer)
        .ok()
        .and_then(|message| message.transcode_to::<LocalApiDiscovery>().ok());
    match discovery {
        Some(discovery) if deserializer.end().is_ok() => LegacyServerRecord::Recorded {
            pid: discovery.pid,
            process_started_at: discovery.process_started_at,
        },
        _ => LegacyServerRecord::Unreadable,
    }
}

fn shape(path: &Path) -> io::Result<Option<EntryShape>> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.is_dir() => Ok(Some(EntryShape::Directory)),
        Ok(metadata) if metadata.is_file() => Ok(Some(EntryShape::File {
            bytes: metadata.len(),
        })),
        Ok(_) => Ok(Some(EntryShape::Other)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn move_entry(source: &Path, destination: &Path) -> io::Result<()> {
    match MoveAction::decide(shape(source)?, shape(destination)?) {
        MoveAction::Skip => Ok(()),
        MoveAction::Rename => fs::rename(source, destination),
        MoveAction::MergeDirectory => {
            for entry in fs::read_dir(source)? {
                let entry = entry?;
                move_entry(&entry.path(), &destination.join(entry.file_name()))?;
            }
            fs::remove_dir(source)
        }
        MoveAction::Refuse => Err(io::Error::other(format!(
            "relocation would overwrite {}",
            destination.display()
        ))),
    }
}

#[cfg(test)]
#[path = "data_home_test.rs"]
mod data_home_tests;
