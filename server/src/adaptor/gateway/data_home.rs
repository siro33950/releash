use crate::domain::daemon::data_home::{
    DataEntry, DataEntryKind, DataHome, DataHomeStore, LegacyServer, RelocationSurvey,
    RELOCATION_MARKER,
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
    fn survey(&self, home: &DataHome) -> Result<RelocationSurvey, TechnicalFailure> {
        let legacy = home.legacy.as_deref();
        Ok(RelocationSurvey {
            legacy_entries: legacy
                .map(entries)
                .transpose()
                .map_err(failure)?
                .unwrap_or_default(),
            current_entries: entries(&home.current).map_err(failure)?,
            legacy_server: legacy.map(server).unwrap_or(LegacyServer::Absent),
            relocation_started: legacy.is_some_and(|path| path.join(RELOCATION_MARKER).exists()),
        })
    }

    fn relocate(&self, home: &DataHome, entries: &[String]) -> Result<(), TechnicalFailure> {
        let Some(legacy) = &home.legacy else {
            return Ok(());
        };
        let parent = home
            .current
            .parent()
            .ok_or_else(|| failure(io::Error::other("missing data directory parent")))?;
        fs::create_dir_all(parent).map_err(failure)?;
        let lock_name = format!(
            ".{}.relocation.lock",
            home.current
                .file_name()
                .ok_or_else(|| failure(io::Error::other("missing data directory name")))?
                .to_string_lossy()
        );
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(parent.join(lock_name))
            .map_err(failure)?;
        fs2::FileExt::lock_exclusive(&lock).map_err(failure)?;
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
        for name in entries {
            if name == RELOCATION_MARKER {
                continue;
            }
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
            let metadata = entry.path().symlink_metadata()?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| io::Error::other("non-UTF-8 relocation entry"))?;
            Ok(DataEntry {
                name,
                kind: if metadata.is_dir() {
                    DataEntryKind::Directory
                } else {
                    DataEntryKind::File
                },
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
    let bytes = match fs::read(path.join("client-api.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return LegacyServer::Absent,
        Err(_) => return LegacyServer::Unverifiable,
    };
    let descriptor = crate::infrastructure::client_protocol::descriptor::pool()
        .get_message_by_name("releash.client.v1.LocalApiDiscovery")
        .expect("discovery descriptor");
    let mut deserializer = serde_json::Deserializer::from_slice(&bytes);
    let discovery = prost_reflect::DynamicMessage::deserialize(descriptor, &mut deserializer)
        .ok()
        .and_then(|message| message.transcode_to::<LocalApiDiscovery>().ok());
    let Some(discovery) = discovery else {
        return LegacyServer::Unverifiable;
    };
    if deserializer.end().is_err() || discovery.pid == 0 || discovery.process_started_at == 0 {
        return LegacyServer::Unverifiable;
    }
    let process = lookup_process_start_time(discovery.pid);
    match (process.process_list_available, process.start_time) {
        (true, Some(start)) if start == discovery.process_started_at => LegacyServer::Running,
        (true, None) => LegacyServer::Stopped,
        (true, Some(start)) if start != 0 => LegacyServer::Stopped,
        _ => LegacyServer::Unverifiable,
    }
}

fn move_entry(source: &Path, destination: &Path) -> io::Result<()> {
    let source_metadata = match source.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    match destination.symlink_metadata() {
        Ok(target) if source_metadata.is_dir() && target.is_dir() => {
            for entry in fs::read_dir(source)? {
                let entry = entry?;
                move_entry(&entry.path(), &destination.join(entry.file_name()))?;
            }
            fs::remove_dir(source)
        }
        Ok(target) if target.is_file() && target.len() == 0 && source_metadata.is_file() => {
            fs::rename(source, destination)
        }
        Ok(_) => Err(io::Error::other(format!(
            "relocation would overwrite {}",
            destination.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::rename(source, destination),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
#[path = "data_home_test.rs"]
mod data_home_tests;
