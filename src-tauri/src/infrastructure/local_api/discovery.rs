use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

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
    pub(crate) instance_id: String,
    pub(crate) pid: u32,
    pub(crate) process_started_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProcessStartTimeLookup {
    pub(crate) process_list_available: bool,
    pub(crate) start_time: Option<u64>,
}

pub(crate) fn process_start_time(pid: u32) -> Option<u64> {
    lookup_process_start_time(pid)
        .start_time
        .filter(|start_time| *start_time != 0)
}

pub(crate) fn lookup_process_start_time(pid: u32) -> ProcessStartTimeLookup {
    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    let start_time = system.process(pid).map(|process| process.start_time());
    if start_time.is_some_and(|start_time| start_time != 0) {
        return ProcessStartTimeLookup {
            process_list_available: true,
            start_time,
        };
    }

    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    ProcessStartTimeLookup {
        process_list_available: !system.processes().is_empty(),
        start_time: system.process(pid).map(|process| process.start_time()),
    }
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
#[path = "discovery_test.rs"]
mod discovery_tests;
