use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::sync::Notify;

use super::child_process;

static CLEANUP_ACTIVE: AtomicBool = AtomicBool::new(false);
static CLEANUP_NOTIFY: OnceLock<Notify> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PidFileV1 {
    pub version: u8,
    pub session_id: String,
    pub backend_id: String,
    pub pid: u32,
    pub pgid: i32,
    #[serde(default)]
    pub owner_app_pid: Option<u32>,
    #[serde(default)]
    pub owner_start_time: Option<u64>,
    pub created_at_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PidRegistration {
    path: PathBuf,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CleanupReport {
    pub scanned: usize,
    pub processed: usize,
    pub skipped: usize,
    pub failures: usize,
}

impl CleanupReport {
    fn failed(&self) -> bool {
        self.failures > 0
    }

    fn merge(&mut self, other: CleanupReport) {
        self.scanned += other.scanned;
        self.processed += other.processed;
        self.skipped += other.skipped;
        self.failures += other.failures;
    }
}

impl PidRegistration {
    pub(crate) fn remove(&self) {
        if let Err(error) = std::fs::remove_file(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!(
                    "failed to remove agent process pid file {}: {error}",
                    self.path.display()
                );
            }
        }
    }
}

pub(crate) async fn wait_for_cleanup_gate() {
    loop {
        let notified = cleanup_notify().notified();
        if !CLEANUP_ACTIVE.load(Ordering::Acquire) {
            break;
        }
        notified.await;
    }
}

pub(crate) fn cleanup_orphan_processes(data_dir: &Path) -> CleanupReport {
    if CLEANUP_ACTIVE.swap(true, Ordering::AcqRel) {
        return CleanupReport {
            skipped: 1,
            ..CleanupReport::default()
        };
    }
    let report = cleanup_orphan_processes_all_dirs(data_dir);
    CLEANUP_ACTIVE.store(false, Ordering::Release);
    cleanup_notify().notify_waiters();
    crate::other::telemetry::record_orphan_cleanup_counts(
        report.scanned,
        report.processed,
        report.skipped,
        report.failures,
        report.failed(),
    );
    report
}

fn cleanup_orphan_processes_all_dirs(data_dir: &Path) -> CleanupReport {
    let mut report = cleanup_orphan_processes_inner(data_dir);
    if let Some(env_data_dir) = data_dir_from_env() {
        if env_data_dir != data_dir {
            report.merge(cleanup_orphan_processes_inner(&env_data_dir));
        }
    }
    report
}

fn cleanup_orphan_processes_inner(data_dir: &Path) -> CleanupReport {
    let mut report = CleanupReport::default();
    let dir = registry_dir(data_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return report;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        report.scanned += 1;
        match read_pid_file(&path) {
            Ok(pid_file) => {
                match owner_status(&pid_file) {
                    OwnerStatus::Live | OwnerStatus::Unknown => {
                        report.skipped += 1;
                        continue;
                    }
                    OwnerStatus::Stale => {}
                }
                if terminate_registered_group(&pid_file) {
                    report.processed += 1;
                } else {
                    report.failures += 1;
                }
                if let Err(error) = std::fs::remove_file(&path) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        report.failures += 1;
                        log::warn!("failed to remove pid file {}: {error}", path.display());
                    }
                }
            }
            Err(error) => {
                report.skipped += 1;
                log::warn!(
                    "skipping invalid agent process pid file {}: {error}",
                    path.display()
                );
            }
        }
    }
    report
}

pub(crate) fn save_pgid(
    data_dir: Option<&Path>,
    session_id: &str,
    backend_id: &str,
    pid: u32,
) -> Option<PidRegistration> {
    let fallback_data_dir = data_dir_from_env();
    let data_dir = match data_dir {
        Some(data_dir) => data_dir,
        None => match fallback_data_dir.as_deref() {
            Some(data_dir) => data_dir,
            None => {
                log::debug!(
                    "skipping agent process pid registration: data_dir and RELEASH_DATA_DIR are unset"
                );
                return None;
            }
        },
    };
    let dir = registry_dir(data_dir);
    if let Err(error) = std::fs::create_dir_all(&dir) {
        log::warn!(
            "failed to create agent process registry {}: {error}",
            dir.display()
        );
        return None;
    }

    let file = PidFileV1 {
        version: 1,
        session_id: session_id.to_string(),
        backend_id: backend_id.to_string(),
        pid,
        pgid: pid as i32,
        owner_app_pid: Some(std::process::id()),
        owner_start_time: current_process_start_time(),
        created_at_ms: now_ms(),
    };
    let path = pid_file_path(&dir, session_id, backend_id, pid);
    match write_pid_file(&path, &file) {
        Ok(()) => Some(PidRegistration { path }),
        Err(error) => {
            log::warn!(
                "failed to write agent process pid file {}: {error}",
                path.display()
            );
            None
        }
    }
}

fn cleanup_notify() -> &'static Notify {
    CLEANUP_NOTIFY.get_or_init(Notify::new)
}

fn registry_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("agent-processes")
}

fn data_dir_from_env() -> Option<PathBuf> {
    std::env::var_os("RELEASH_DATA_DIR").map(PathBuf::from)
}

fn pid_file_path(dir: &Path, session_id: &str, backend_id: &str, pid: u32) -> PathBuf {
    let name = format!(
        "{}.{}.{}.json",
        sanitize_file_component(session_id),
        sanitize_file_component(backend_id),
        pid
    );
    dir.join(name)
}

fn sanitize_file_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

fn write_pid_file(path: &Path, file: &PidFileV1) -> Result<(), String> {
    let json = serde_json::to_string_pretty(file).map_err(|error| error.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)
        .map_err(|error| format!("failed to write pid tmp {}: {error}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .map_err(|error| format!("failed to rename pid file {}: {error}", path.display()))
}

fn read_pid_file(path: &Path) -> Result<PidFileV1, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read pid file: {error}"))?;
    let file: PidFileV1 =
        serde_json::from_str(&content).map_err(|error| format!("invalid pid file: {error}"))?;
    if file.version != 1 {
        return Err(format!("unsupported pid file version {}", file.version));
    }
    if file.pgid <= 1 {
        return Err(format!("unsafe process group {}", file.pgid));
    }
    Ok(file)
}

fn terminate_registered_group(file: &PidFileV1) -> bool {
    #[cfg(unix)]
    {
        let term = child_process::signal_process_group(file.pgid, libc::SIGTERM);
        std::thread::sleep(std::time::Duration::from_millis(50));
        let kill = child_process::signal_process_group(file.pgid, libc::SIGKILL);
        term.is_ok() && kill.is_ok()
    }

    #[cfg(not(unix))]
    {
        let _ = file;
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnerStatus {
    Live,
    Stale,
    Unknown,
}

fn owner_status(file: &PidFileV1) -> OwnerStatus {
    let (Some(owner_pid), Some(owner_start_time)) = (file.owner_app_pid, file.owner_start_time)
    else {
        return OwnerStatus::Unknown;
    };
    if owner_start_time == 0 {
        return OwnerStatus::Unknown;
    }
    match process_start_time(owner_pid) {
        Some(start_time) if start_time == owner_start_time => OwnerStatus::Live,
        Some(_) => OwnerStatus::Stale,
        None => OwnerStatus::Stale,
    }
}

fn current_process_start_time() -> Option<u64> {
    process_start_time(std::process::id())
}

fn process_start_time(pid: u32) -> Option<u64> {
    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    system
        .process(pid)
        .map(|process| process.start_time())
        .filter(|start_time| *start_time > 0)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_pgid_登録ファイルを作成し_removeで削除する() {
        let tmp = tempfile::tempdir().unwrap();
        let registration =
            save_pgid(Some(tmp.path()), "session/1", "claude", 42).expect("pid registration");
        assert!(registration.path.exists());

        let file = read_pid_file(&registration.path).unwrap();
        assert_eq!(file.session_id, "session/1");
        assert_eq!(file.backend_id, "claude");
        assert_eq!(file.pid, 42);
        assert_eq!(file.pgid, 42);
        assert_eq!(file.owner_app_pid, Some(std::process::id()));
        assert!(file.owner_start_time.unwrap_or_default() > 0);

        registration.remove();
        assert!(!registration.path.exists());
    }

    #[test]
    fn test_cleanup_orphan_processes_invalid_fileをskipする() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = registry_dir(tmp.path());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bad.json"), "{").unwrap();

        let report = cleanup_orphan_processes_inner(tmp.path());

        assert_eq!(report.scanned, 1);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.processed, 0);
        assert_eq!(report.failures, 0);
    }

    #[test]
    fn test_cleanup_orphan_processes_owner生存中のpid_fileはskipする() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = registry_dir(tmp.path());
        std::fs::create_dir_all(&dir).unwrap();
        let file = PidFileV1 {
            version: 1,
            session_id: "session".to_string(),
            backend_id: "claude".to_string(),
            pid: 999_999,
            pgid: 999_999,
            owner_app_pid: Some(std::process::id()),
            owner_start_time: current_process_start_time(),
            created_at_ms: now_ms(),
        };
        let path = dir.join("live-owner.json");
        write_pid_file(&path, &file).unwrap();

        let report = cleanup_orphan_processes_inner(tmp.path());

        assert_eq!(report.scanned, 1);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.processed, 0);
        assert_eq!(report.failures, 0);
        assert!(path.exists());
    }

    #[test]
    fn test_cleanup_orphan_processes_owner_pid不在のpid_fileは処理する() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = registry_dir(tmp.path());
        std::fs::create_dir_all(&dir).unwrap();
        let file = PidFileV1 {
            version: 1,
            session_id: "session".to_string(),
            backend_id: "claude".to_string(),
            pid: 999_999,
            pgid: i32::MAX,
            owner_app_pid: Some(u32::MAX),
            owner_start_time: Some(1),
            created_at_ms: now_ms(),
        };
        let path = dir.join("stale-owner.json");
        write_pid_file(&path, &file).unwrap();

        let report = cleanup_orphan_processes_inner(tmp.path());

        assert_eq!(report.scanned, 1);
        assert_eq!(report.skipped, 0);
        assert_eq!(report.processed, 1);
        assert_eq!(report.failures, 0);
        assert!(!path.exists());
    }

    #[test]
    fn test_owner_status_検証不能なら_unknown() {
        let file = PidFileV1 {
            version: 1,
            session_id: "session".to_string(),
            backend_id: "claude".to_string(),
            pid: 42,
            pgid: 42,
            owner_app_pid: None,
            owner_start_time: None,
            created_at_ms: now_ms(),
        };

        assert_eq!(owner_status(&file), OwnerStatus::Unknown);
    }

    #[test]
    fn test_owner_status_owner_pid不在なら_stale() {
        let file = PidFileV1 {
            version: 1,
            session_id: "session".to_string(),
            backend_id: "claude".to_string(),
            pid: 42,
            pgid: 42,
            owner_app_pid: Some(u32::MAX),
            owner_start_time: Some(1),
            created_at_ms: now_ms(),
        };

        assert_eq!(owner_status(&file), OwnerStatus::Stale);
    }
}
