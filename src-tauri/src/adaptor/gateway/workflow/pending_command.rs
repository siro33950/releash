//! [06] CLI mutating CLI 経路の file-direct 仲介層。
//!
//! `releash workflow approve|reject|abort` CLI は engine と直接 IPC せず、
//! pending command を本モジュールが管理する file ディレクトリへ書き出す。
//! 稼働中アプリの watcher が pickup し、dispatcher adapter 経由で engine runtime
//! primitive に渡す（spec [06] アーキテクチャ概要 / 責務配置）。
//!
//! 本モジュールの責務:
//! - pending payload の typed 表現と JSON serde 境界
//! - file 書き込み (CLI 側) / 列挙 (watcher 側) / 処理済みマーキング (dispatcher 側)
//! - TTL に基づく古い pending entry の cleanup
//!
//! 担当しない: engine 内部 state mutation、runtime primitive 呼び出し、
//! event 発行（これらは dispatcher adapter / engine 側）。

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::io;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// pending command のディレクトリ名（`<data_dir>/workflow_pending/`）。
const PENDING_DIR_NAME: &str = "workflow_pending";
/// 未処理キューのサブディレクトリ。
const PENDING_SUBDIR: &str = "pending";
/// 処理済みエントリの退避先。
const PROCESSED_SUBDIR: &str = "processed";
/// dispatch claim 済みエントリの退避先。
const PROCESSING_SUBDIR: &str = "processing";
/// pending command file の拡張子。
const PENDING_EXT: &str = "json";
const MAX_REQUESTED_AT_FUTURE_SKEW_SECS: f64 = 5.0 * 60.0;
const MAX_PENDING_COMMAND_FILE_BYTES: u64 = 64 * 1024;
const PROCESSING_ORPHAN_GRACE_SECS: f64 = 30.0;
/// 既定の TTL: 24 時間。古い未処理要求はこれを超えると engine に到達しない
/// （spec [06] Rule: 古い未処理要求は無期限に滞留しない）。
pub const DEFAULT_PENDING_TTL_SECS: f64 = 24.0 * 60.0 * 60.0;

/// [06] CLI が要求した mutation の pending file 永続化 schema。
///
/// event log の観測 schema (`CliMutationRequestRecord`) とは owner 境界を分ける。
/// dispatcher adapter が本 payload を engine runtime primitive 入力と event log 用
/// request の両方へ明示変換する。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PendingCommandPayload {
    Approve {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        node_name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        comment: Option<String>,
    },
    Reject {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        node_name: Option<String>,
        reason: String,
    },
    Abort {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        node_name: Option<String>,
    },
    /// [08] `releash workflow output submit` 経由で書き出される構造化出力提出。
    /// engine 側 dispatcher が submit-output runtime primitive に変換する。
    SubmitOutput {
        step_name: String,
        contract: String,
        structured_output: serde_json::Value,
    },
}

#[cfg(test)]
pub type CliRequestPayload = PendingCommandPayload;

/// CLI が書き出す pending command 1 件分の永続表現。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingCommand {
    /// この pending entry のユニーク ID（重複検知 / 処理済みマーキング用途）。
    pub id: String,
    /// 対象 run の run_id（UUID）。
    pub run_id: String,
    pub payload: PendingCommandPayload,
    /// CLI が pending command を書き出した時刻（Unix 秒）。TTL 判定と
    /// `CliMutationRequested.requested_at` の値として再利用される。
    pub requested_at: f64,
}

impl PendingCommand {
    /// 新規 pending entry を生成する。`id` は内部で UUID v4 を払い出す。
    #[cfg(test)]
    pub fn new(run_id: String, payload: PendingCommandPayload, requested_at: f64) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            run_id,
            payload,
            requested_at,
        }
    }
}

/// 列挙された pending entry。dispatcher adapter は本値を介して
/// 「処理済みマーキング」を要求する（spec [06] 一度だけ処理境界）。
#[derive(Debug, Clone)]
pub struct PendingCommandEntry {
    pub command: PendingCommand,
    /// store 管理下の file path。claim 前は `pending/`。
    pub path: PathBuf,
}

/// claim 後に `processing/` 配下へ移動済みであることを型で表す entry。
#[derive(Debug, Clone)]
pub struct ProcessingCommandEntry {
    pub command: PendingCommand,
    /// store 管理下の file path。claim 後は `processing/`。
    pub path: PathBuf,
}

/// dispatch 中の processing entry と、その entry を守る advisory lock。
pub struct PendingCommandClaim {
    pub entry: ProcessingCommandEntry,
    _lock: ProcessingEntryLock,
}

struct ProcessingEntryLock {
    _file: File,
}

impl ProcessingEntryLock {
    fn acquire(path: &Path) -> io::Result<Self> {
        let lock_path = processing_lock_path(path)?;
        let file = open_processing_lock_file(&lock_path)?;
        lock_file_exclusive(&file)?;
        Ok(Self { _file: file })
    }

    fn try_acquire(path: &Path) -> io::Result<Option<Self>> {
        let lock_path = processing_lock_path(path)?;
        let file = open_processing_lock_file(&lock_path)?;
        match try_lock_file_exclusive(&file) {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// [06] pending command store: file-direct 仲介層の永続化アダプタ。
///
/// `data_dir` 配下に `workflow_pending/{pending,processed}/` を作る。CLI は
/// `pending/` に typed payload を書き出し、watcher / dispatcher adapter は
/// `pending/` を列挙して `processed/` に移動する。
pub struct PendingCommandStore {
    pending_dir: PathBuf,
    processing_dir: PathBuf,
    processed_dir: PathBuf,
}

impl PendingCommandStore {
    /// data_dir 配下に pending command 用のディレクトリを構える。
    pub fn new(data_dir: &Path) -> Self {
        let base = data_dir.join(PENDING_DIR_NAME);
        Self {
            pending_dir: base.join(PENDING_SUBDIR),
            processing_dir: base.join(PROCESSING_SUBDIR),
            processed_dir: base.join(PROCESSED_SUBDIR),
        }
    }

    /// CLI 側 writer が watch すべき pending ディレクトリ。
    pub fn pending_dir(&self) -> &Path {
        &self.pending_dir
    }

    pub(crate) fn ensure_dirs(&self) -> io::Result<()> {
        ensure_secure_dir(&self.pending_dir)?;
        ensure_secure_dir(&self.processing_dir)?;
        ensure_secure_dir(&self.processed_dir)?;
        Ok(())
    }

    fn entry_path(&self, dir: &Path, id: &str) -> PathBuf {
        dir.join(format!("{id}.{PENDING_EXT}"))
    }

    /// [06] CLI 完了基準境界: pending command を atomic rename で `pending/` に
    /// 書き出す。書き出し完了時点で CLI は「受理キュー投入完了」とみなす
    /// （spec [06] CLI 完了基準境界）。
    pub fn write_pending(&self, command: &PendingCommand) -> io::Result<PathBuf> {
        self.ensure_dirs()?;
        validate_command_id(&command.id)?;
        let final_path = self.entry_path(&self.pending_dir, &command.id);
        // tmp file に書き込んでから rename することで、watcher が partial write を
        // pickup しないようにする。
        let tmp_path = self
            .pending_dir
            .join(format!(".{}.{}.tmp", command.id, std::process::id()));
        let json = serde_json::to_vec_pretty(command)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if json.len() as u64 > MAX_PENDING_COMMAND_FILE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "pending command file exceeds size limit",
            ));
        }
        write_secure_file(&tmp_path, &json)?;
        if let Err(e) = fs::rename(&tmp_path, &final_path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e);
        }
        ensure_owner_read_write_file(&final_path)?;
        Ok(final_path)
    }

    /// `pending/` 配下の全 entry を読み込む。JSON parse に失敗した file は無視して
    /// warn log を残す（caller が個別エラー処理を必要としない範囲で続行する境界）。
    pub fn list_pending(&self) -> io::Result<Vec<PendingCommandEntry>> {
        if !self.pending_dir.exists() {
            return Ok(Vec::new());
        }
        validate_owner_only_dir(&self.pending_dir)?;
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.pending_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !is_candidate_pending_file(&path) {
                continue;
            }
            match read_pending_file(&path) {
                Ok(command)
                    if validate_command_id(&command.id).is_ok()
                        && validate_run_id(&command.run_id).is_ok() =>
                {
                    entries.push(PendingCommandEntry { command, path })
                }
                Ok(command) => {
                    log::warn!(
                        "pending command file skipped (invalid id or run_id): {} (id={}, run_id={})",
                        path.display(),
                        command.id,
                        command.run_id
                    );
                }
                Err(e) => {
                    log::warn!(
                        "pending command file skipped (parse error): {} ({e})",
                        path.display()
                    );
                }
            }
        }
        // 並び順は requested_at 昇順（FIFO に近い処理順を担保）。
        entries.sort_by(|a, b| {
            a.command
                .requested_at
                .partial_cmp(&b.command.requested_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(entries)
    }

    /// dispatch 前の atomic claim 境界。rename に成功した caller だけが dispatch する。
    pub fn claim_pending(
        &self,
        entry: &PendingCommandEntry,
    ) -> io::Result<Option<PendingCommandClaim>> {
        self.ensure_dirs()?;
        validate_command_id(&entry.command.id)?;
        let file_name = safe_entry_file_name(&entry.path)?;
        let claimed_path = self.processing_dir.join(file_name);
        match fs::rename(&entry.path, &claimed_path) {
            Ok(()) => {
                let lock = match ProcessingEntryLock::acquire(&claimed_path) {
                    Ok(lock) => lock,
                    Err(e) => {
                        let _ = fs::rename(&claimed_path, &entry.path);
                        return Err(e);
                    }
                };
                Ok(Some(PendingCommandClaim {
                    entry: ProcessingCommandEntry {
                        command: entry.command.clone(),
                        path: claimed_path,
                    },
                    _lock: lock,
                }))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// retryable failure 時に claim を pending へ戻す。
    pub fn release_claim(&self, entry: &ProcessingCommandEntry) -> io::Result<()> {
        self.ensure_dirs()?;
        validate_command_id(&entry.command.id)?;
        self.validate_processing_entry(entry)?;
        let file_name = safe_entry_file_name(&entry.path)?;
        let pending_path = self.pending_dir.join(file_name);
        match fs::rename(&entry.path, pending_path) {
            Ok(()) => {
                remove_processing_lock_file(&entry.path);
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// [06] 一度だけ処理境界: dispatcher adapter は dispatch 完了後に本メソッドで
    /// claimed entry を `processed/` に移動する。同一 file の二重検知 / watcher
    /// 再発火は本境界の内側で吸収される（spec [06] 一度だけ処理境界）。
    pub fn mark_processed(&self, entry: &ProcessingCommandEntry) -> io::Result<()> {
        self.ensure_dirs()?;
        validate_command_id(&entry.command.id)?;
        self.validate_processing_entry(entry)?;
        let file_name = safe_entry_file_name(&entry.path)?;
        let target = self.processed_dir.join(file_name);
        match fs::rename(&entry.path, &target) {
            Ok(()) => {
                remove_processing_lock_file(&entry.path);
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                // 既に処理済み（並行 watcher 等）。冪等な挙動を保証する。
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// 前回プロセスが claim 後に落ちた可能性がある `processing/` の orphan を、claim
    /// file の mtime が十分古い場合に限って `pending/` に戻す。新しい processing file は
    /// 別 watcher / 別プロセスが dispatch 中の claim とみなし、同一 request の並行 dispatch
    /// を避ける。
    pub fn requeue_unexpired_processing(&self, now: f64, ttl_secs: f64) -> io::Result<usize> {
        #[cfg(not(unix))]
        {
            let _ = (now, ttl_secs);
            return Ok(0);
        }
        #[cfg(unix)]
        {
            if !self.processing_dir.exists() {
                return Ok(0);
            }
            self.ensure_dirs()?;
            validate_owner_only_dir(&self.processing_dir)?;
            let mut moved = 0usize;
            for entry in fs::read_dir(&self.processing_dir)? {
                let path = entry?.path();
                if path.extension().and_then(OsStr::to_str) != Some(PENDING_EXT) {
                    continue;
                }
                if !is_processing_orphan_candidate(&path, now)? {
                    continue;
                }
                let Some(_lock) = ProcessingEntryLock::try_acquire(&path)? else {
                    continue;
                };
                let command = match read_pending_file(&path) {
                    Ok(command) => command,
                    Err(e) => {
                        log::warn!(
                            "processing command file skipped during orphan requeue: {} ({e})",
                            path.display()
                        );
                        continue;
                    }
                };
                let file_name = safe_entry_file_name(&path)?;
                let target_dir = if is_expired_or_clock_skewed(command.requested_at, now, ttl_secs)
                {
                    &self.processed_dir
                } else {
                    &self.pending_dir
                };
                match fs::rename(&path, target_dir.join(file_name)) {
                    Ok(()) => {
                        remove_processing_lock_file(&path);
                        moved += 1;
                    }
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
            }
            Ok(moved)
        }
    }

    /// [06] TTL / cleanup 境界: `now` 時刻基準で `ttl_secs` を超えた pending entry
    /// を pickup 対象から除外する（spec [06] 古い未処理要求は engine に到達しない）。
    /// 除外手段は file 削除（処理済みではなく失効扱い）として表す。
    pub fn cleanup_expired(&self, now: f64, ttl_secs: f64) -> io::Result<usize> {
        if !self.pending_dir.exists() {
            return Ok(0);
        }
        validate_owner_only_dir(&self.pending_dir)?;
        let mut removed = 0usize;
        for entry in fs::read_dir(&self.pending_dir)? {
            let path = entry?.path();
            if !is_candidate_pending_file(&path) {
                continue;
            }
            let should_remove = match read_pending_file(&path) {
                Ok(command) => {
                    validate_command_id(&command.id).is_err()
                        || validate_run_id(&command.run_id).is_err()
                        || is_expired_or_clock_skewed(command.requested_at, now, ttl_secs)
                }
                Err(e) => {
                    log::warn!(
                        "removing unreadable pending command during cleanup: {} ({e})",
                        path.display()
                    );
                    true
                }
            };
            if should_remove {
                match fs::remove_file(&path) {
                    Ok(()) => removed += 1,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => {
                        log::warn!(
                            "failed to remove expired pending command {}: {e}",
                            path.display()
                        );
                    }
                }
            }
        }
        Ok(removed)
    }

    fn validate_processing_entry(&self, entry: &ProcessingCommandEntry) -> io::Result<()> {
        if entry.path.parent() != Some(self.processing_dir.as_path()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "pending command claim must refer to processing directory",
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn gc_delete_paths_for_run(&self, run_id: &str) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for dir in [&self.pending_dir, &self.processing_dir, &self.processed_dir] {
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !is_candidate_pending_file(&path) {
                    continue;
                }
                match read_pending_file(&path) {
                    Ok(command) if command.run_id == run_id => paths.push(path),
                    Ok(_) => {}
                    Err(error) => {
                        log::warn!(
                            "app data gc skipped unreadable pending workflow command {}: {error}",
                            path.display()
                        );
                    }
                }
            }
        }
        paths
    }

    pub(crate) fn gc_delete_paths_by_run(&self) -> HashMap<String, Vec<PathBuf>> {
        let mut paths_by_run: HashMap<String, Vec<PathBuf>> = HashMap::new();
        for dir in [&self.pending_dir, &self.processing_dir, &self.processed_dir] {
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !is_candidate_pending_file(&path) {
                    continue;
                }
                match read_pending_file(&path) {
                    Ok(command) => {
                        paths_by_run.entry(command.run_id).or_default().push(path);
                    }
                    Err(error) => {
                        log::warn!(
                            "app data gc skipped unreadable pending workflow command {}: {error}",
                            path.display()
                        );
                    }
                }
            }
        }
        paths_by_run
    }
}

fn read_pending_file(path: &Path) -> io::Result<PendingCommand> {
    let link_metadata = fs::symlink_metadata(path)?;
    validate_owner_read_write_file_metadata(&link_metadata)?;
    if !link_metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pending command path must be a regular file",
        ));
    }
    if link_metadata.len() > MAX_PENDING_COMMAND_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pending command file exceeds size limit",
        ));
    }
    let file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    validate_owner_read_write_file_metadata(&metadata)?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pending command path must be a regular file",
        ));
    }
    if metadata.len() > MAX_PENDING_COMMAND_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pending command file exceeds size limit",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_PENDING_COMMAND_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PENDING_COMMAND_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pending command file exceeds size limit",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn ensure_secure_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    ensure_owner_only_dir(path)
}

#[cfg(unix)]
fn ensure_owner_only_dir(path: &Path) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    validate_owner_only_dir(path)
}

#[cfg(not(unix))]
fn ensure_owner_only_dir(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn validate_owner_only_dir(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pending command directory must be a directory",
        ));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "pending command directory must be owner-only",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_owner_only_dir(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn write_secure_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn write_secure_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    fs::write(path, bytes)
}

fn ensure_owner_read_write_file(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(unix)]
fn validate_owner_read_write_file_metadata(metadata: &fs::Metadata) -> io::Result<()> {
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "pending command file must be owner read/write only",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_owner_read_write_file_metadata(_metadata: &fs::Metadata) -> io::Result<()> {
    Ok(())
}

fn is_candidate_pending_file(path: &Path) -> bool {
    path.extension().and_then(OsStr::to_str) == Some(PENDING_EXT)
        && !path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|n| n.starts_with('.'))
}

fn validate_command_id(id: &str) -> io::Result<()> {
    Uuid::parse_str(id).map(|_| ()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "pending command id must be UUID",
        )
    })
}

fn validate_run_id(run_id: &str) -> io::Result<()> {
    Uuid::parse_str(run_id).map(|_| ()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "pending command run_id must be UUID",
        )
    })
}

fn safe_entry_file_name(path: &Path) -> io::Result<&OsStr> {
    path.file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid pending file name"))
}

fn processing_lock_path(path: &Path) -> io::Result<PathBuf> {
    let file_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid pending file name"))?;
    Ok(path.with_file_name(format!("{file_name}.lock")))
}

fn remove_processing_lock_file(path: &Path) {
    if let Ok(lock_path) = processing_lock_path(path) {
        let _ = fs::remove_file(lock_path);
    }
}

#[cfg(unix)]
fn open_processing_lock_file(path: &Path) -> io::Result<File> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_processing_lock_file(path: &Path) -> io::Result<File> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

#[cfg(unix)]
fn lock_file_exclusive(file: &File) -> io::Result<()> {
    flock(file, libc::LOCK_EX)
}

#[cfg(unix)]
fn try_lock_file_exclusive(file: &File) -> io::Result<()> {
    flock(file, libc::LOCK_EX | libc::LOCK_NB)
}

#[cfg(unix)]
fn flock(file: &File, operation: libc::c_int) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    let result = unsafe { libc::flock(file.as_raw_fd(), operation) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn lock_file_exclusive(_file: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(not(unix))]
fn try_lock_file_exclusive(_file: &File) -> io::Result<()> {
    Ok(())
}

fn is_expired_or_clock_skewed(requested_at: f64, now: f64, ttl_secs: f64) -> bool {
    requested_at > now + MAX_REQUESTED_AT_FUTURE_SKEW_SECS || now - requested_at > ttl_secs
}

fn is_processing_orphan_candidate(path: &Path, now: f64) -> io::Result<bool> {
    let modified = fs::metadata(path)?.modified()?;
    let modified_at = modified
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(now);
    Ok(now - modified_at >= PROCESSING_ORPHAN_GRACE_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn approve_payload() -> CliRequestPayload {
        CliRequestPayload::Approve {
            node_name: Some("review".to_string()),
            comment: Some("looks good".to_string()),
        }
    }

    fn reject_payload() -> CliRequestPayload {
        CliRequestPayload::Reject {
            node_name: None,
            reason: "must rework".to_string(),
        }
    }

    fn abort_payload(node: Option<&str>) -> CliRequestPayload {
        CliRequestPayload::Abort {
            node_name: node.map(str::to_string),
        }
    }

    fn test_uuid(n: u128) -> String {
        uuid::Uuid::from_u128(n).to_string()
    }

    fn unix_now() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
    }

    /// spec [06] Rule: CLI 完了基準は「受理キュー投入」までで統一する。
    /// 書き出した pending entry が `pending/` から読み出せることを担保する。
    #[test]
    fn write_pending_persists_entry_readable_via_list_pending() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        let cmd = PendingCommand::new(test_uuid(1), approve_payload(), 100.0);
        let path = store.write_pending(&cmd).unwrap();
        assert!(path.exists(), "pending file must exist");

        let entries = store.list_pending().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, cmd);
    }

    /// spec [06] Rule: 各 state 変化要求は engine により一度だけ処理される。
    /// `mark_processed` で entry を pending から除外し、再列挙対象から外れる。
    #[test]
    fn mark_processed_moves_entry_out_of_pending_queue() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        let cmd = PendingCommand::new(test_uuid(2), reject_payload(), 200.0);
        store.write_pending(&cmd).unwrap();
        let entry = store.list_pending().unwrap().pop().unwrap();
        let claimed = store.claim_pending(&entry).unwrap().unwrap();
        store.mark_processed(&claimed.entry).unwrap();
        let after = store.list_pending().unwrap();
        assert!(
            after.is_empty(),
            "processed entry must be excluded from pickup"
        );
        // 冪等性: 既に移動済みの entry に対しても Err を返さない。
        store.mark_processed(&claimed.entry).unwrap();
    }

    /// spec [06] Rule: 古い未処理要求は無期限に滞留せず engine による処理対象から除外される。
    #[test]
    fn cleanup_expired_removes_entries_beyond_ttl() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        let old = PendingCommand::new(test_uuid(3), abort_payload(None), 100.0);
        let fresh = PendingCommand::new(test_uuid(4), abort_payload(None), 950.0);
        store.write_pending(&old).unwrap();
        store.write_pending(&fresh).unwrap();

        let removed = store.cleanup_expired(1000.0, 100.0).unwrap();
        assert_eq!(removed, 1, "only the aged entry should be removed");
        let remaining = store.list_pending().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].command.run_id, test_uuid(4));
    }

    /// spec [06] Rule: pending payload の typed shape は pending file の owner 境界で
    /// 独立しており、JSON serde で round-trip が成立する。
    #[test]
    fn payload_round_trips_via_json() {
        for p in [
            approve_payload(),
            reject_payload(),
            abort_payload(Some("review")),
            abort_payload(None),
        ] {
            let cmd = PendingCommand::new(test_uuid(10), p.clone(), 1.0);
            let json = serde_json::to_string(&cmd).unwrap();
            let back: PendingCommand = serde_json::from_str(&json).unwrap();
            assert_eq!(back, cmd);
        }
    }

    /// list_pending は requested_at 昇順で返す（pickup 順を caller が保証できる境界）。
    #[test]
    fn list_pending_orders_by_requested_at_ascending() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        store
            .write_pending(&PendingCommand::new(
                test_uuid(5),
                abort_payload(None),
                300.0,
            ))
            .unwrap();
        store
            .write_pending(&PendingCommand::new(
                test_uuid(6),
                abort_payload(None),
                100.0,
            ))
            .unwrap();
        store
            .write_pending(&PendingCommand::new(
                test_uuid(7),
                abort_payload(None),
                200.0,
            ))
            .unwrap();
        let entries = store.list_pending().unwrap();
        let ats: Vec<f64> = entries.iter().map(|e| e.command.requested_at).collect();
        assert_eq!(ats, vec![100.0, 200.0, 300.0]);
    }

    /// 不正な JSON が混じっても list_pending は他 entry を返し、エラー伝播しない。
    #[test]
    fn list_pending_skips_unparseable_entries() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        store
            .write_pending(&PendingCommand::new(test_uuid(8), abort_payload(None), 1.0))
            .unwrap();
        // garbage file を pending dir に置く。
        let garbage = store.pending_dir().join("garbage.json");
        std::fs::write(&garbage, b"not json").unwrap();
        let entries = store.list_pending().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command.run_id, test_uuid(8));
    }

    #[test]
    fn claim_pending_allows_only_one_consumer_and_moves_to_processing() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        let cmd = PendingCommand::new(test_uuid(9), abort_payload(None), 1.0);
        store.write_pending(&cmd).unwrap();
        let entry = store.list_pending().unwrap().pop().unwrap();

        let claimed = store.claim_pending(&entry).unwrap().unwrap();
        assert!(
            store.claim_pending(&entry).unwrap().is_none(),
            "second consumer must not be able to claim the same file"
        );
        assert!(store.list_pending().unwrap().is_empty());
        store.mark_processed(&claimed.entry).unwrap();
        assert!(store.list_pending().unwrap().is_empty());
    }

    #[test]
    fn mark_processed_rejects_unclaimed_pending_entry_shape() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        let cmd = PendingCommand::new(test_uuid(16), abort_payload(None), 1.0);
        store.write_pending(&cmd).unwrap();
        let entry = store.list_pending().unwrap().pop().unwrap();
        let wrong_state_entry = ProcessingCommandEntry {
            command: entry.command,
            path: entry.path,
        };

        let err = store.mark_processed(&wrong_state_entry).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn fresh_processing_entries_within_ttl_are_not_requeued_while_claim_may_be_active() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        let now = unix_now();
        let cmd = PendingCommand::new(test_uuid(11), abort_payload(None), now);
        store.write_pending(&cmd).unwrap();
        let entry = store.list_pending().unwrap().pop().unwrap();
        let _claimed = store.claim_pending(&entry).unwrap().unwrap();

        let moved = store
            .requeue_unexpired_processing(now + PROCESSING_ORPHAN_GRACE_SECS - 1.0, 100.0)
            .unwrap();
        assert_eq!(moved, 0);
        assert!(store.list_pending().unwrap().is_empty());
    }

    #[test]
    fn old_orphaned_processing_entries_within_ttl_are_requeued_for_dispatch() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        let now = unix_now();
        let cmd = PendingCommand::new(test_uuid(111), abort_payload(None), now);
        store.write_pending(&cmd).unwrap();
        let entry = store.list_pending().unwrap().pop().unwrap();
        let claimed = store.claim_pending(&entry).unwrap().unwrap();
        drop(claimed);

        let moved = store
            .requeue_unexpired_processing(now + PROCESSING_ORPHAN_GRACE_SECS + 1.0, 100.0)
            .unwrap();
        assert_eq!(moved, 1);
        let entries = store.list_pending().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command.id, cmd.id);
    }

    #[test]
    fn old_processing_entries_with_active_lock_are_not_requeued() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        let now = unix_now();
        let cmd = PendingCommand::new(test_uuid(112), abort_payload(None), now);
        store.write_pending(&cmd).unwrap();
        let entry = store.list_pending().unwrap().pop().unwrap();
        let _claimed = store.claim_pending(&entry).unwrap().unwrap();

        let moved = store
            .requeue_unexpired_processing(now + PROCESSING_ORPHAN_GRACE_SECS + 1.0, 100.0)
            .unwrap();
        assert_eq!(moved, 0);
        assert!(store.list_pending().unwrap().is_empty());
    }

    #[cfg(not(unix))]
    #[test]
    fn processing_orphan_requeue_is_disabled_without_effective_file_locks() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        let now = unix_now();
        let cmd = PendingCommand::new(test_uuid(113), abort_payload(None), now);
        store.write_pending(&cmd).unwrap();
        let entry = store.list_pending().unwrap().pop().unwrap();
        let claimed = store.claim_pending(&entry).unwrap().unwrap();
        drop(claimed);

        let moved = store
            .requeue_unexpired_processing(now + PROCESSING_ORPHAN_GRACE_SECS + 1.0, 100.0)
            .unwrap();
        assert_eq!(moved, 0);
        assert!(store.list_pending().unwrap().is_empty());
    }

    #[test]
    fn expired_orphaned_processing_entries_are_moved_out_of_pickup() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        let cmd = PendingCommand::new(test_uuid(12), abort_payload(None), 1.0);
        store.write_pending(&cmd).unwrap();
        let entry = store.list_pending().unwrap().pop().unwrap();
        let claimed = store.claim_pending(&entry).unwrap().unwrap();
        drop(claimed);

        let moved = store
            .requeue_unexpired_processing(unix_now() + PROCESSING_ORPHAN_GRACE_SECS + 1.0, 100.0)
            .unwrap();
        assert_eq!(moved, 1);
        assert!(store.list_pending().unwrap().is_empty());
    }

    #[test]
    fn write_pending_rejects_non_uuid_id_to_prevent_path_traversal() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        let mut cmd = PendingCommand::new(test_uuid(13), abort_payload(None), 1.0);
        cmd.id = "../escape".to_string();

        let err = store.write_pending(&cmd).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(!tmp.path().join("escape.json").exists());
    }

    #[test]
    fn cleanup_expired_removes_future_clock_skew_entries() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        let future = PendingCommand::new(test_uuid(14), abort_payload(None), 10_000.0);
        store.write_pending(&future).unwrap();

        let removed = store
            .cleanup_expired(1_000.0, DEFAULT_PENDING_TTL_SECS)
            .unwrap();
        assert_eq!(removed, 1);
        assert!(store.list_pending().unwrap().is_empty());
    }

    #[test]
    fn list_pending_skips_oversized_pending_files() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        store.ensure_dirs().unwrap();
        let oversized = store
            .pending_dir()
            .join(format!("{}.json", uuid::Uuid::new_v4()));
        std::fs::write(
            &oversized,
            vec![b'x'; (MAX_PENDING_COMMAND_FILE_BYTES + 1) as usize],
        )
        .unwrap();

        assert!(store.list_pending().unwrap().is_empty());
    }

    #[test]
    fn cleanup_expired_removes_invalid_entries_skipped_by_list_pending() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        store
            .write_pending(&PendingCommand::new(
                test_uuid(15),
                abort_payload(None),
                1.0,
            ))
            .unwrap();
        let invalid = store
            .pending_dir()
            .join(format!("{}.json", uuid::Uuid::new_v4()));
        std::fs::write(&invalid, b"not json").unwrap();

        let listed = store.list_pending().unwrap();
        assert_eq!(listed.len(), 1);
        let removed = store
            .cleanup_expired(10.0, DEFAULT_PENDING_TTL_SECS)
            .unwrap();
        assert_eq!(removed, 1);
        assert!(
            !invalid.exists(),
            "invalid pending file must not warn forever after cleanup"
        );
        assert_eq!(store.list_pending().unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn list_pending_rejects_symlink_entries_before_reading_target() {
        use std::os::unix::fs::symlink;

        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        store.ensure_dirs().unwrap();
        let target = tmp.path().join("target.json");
        std::fs::write(&target, b"not a pending command").unwrap();
        let link = store
            .pending_dir()
            .join(format!("{}.json", uuid::Uuid::new_v4()));
        symlink(&target, &link).unwrap();

        assert!(store.list_pending().unwrap().is_empty());
        let removed = store
            .cleanup_expired(10.0, DEFAULT_PENDING_TTL_SECS)
            .unwrap();
        assert_eq!(removed, 1);
        assert!(!link.exists(), "symlink must be removed from pending queue");
        assert!(
            target.exists(),
            "cleanup must not follow and remove symlink target"
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_pending_creates_owner_only_directory_and_file_permissions() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        let cmd = PendingCommand::new(test_uuid(17), approve_payload(), 1.0);

        let path = store.write_pending(&cmd).unwrap();

        assert_eq!(
            std::fs::metadata(store.pending_dir())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn list_pending_rejects_group_or_world_writable_pending_directory() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        store
            .write_pending(&PendingCommand::new(
                test_uuid(18),
                abort_payload(None),
                1.0,
            ))
            .unwrap();
        std::fs::set_permissions(store.pending_dir(), std::fs::Permissions::from_mode(0o777))
            .unwrap();

        let err = store.list_pending().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[cfg(unix)]
    #[test]
    fn list_pending_skips_group_or_world_readable_pending_file() {
        let tmp = TempDir::new().unwrap();
        let store = PendingCommandStore::new(tmp.path());
        let path = store
            .write_pending(&PendingCommand::new(
                test_uuid(19),
                abort_payload(None),
                1.0,
            ))
            .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert!(store.list_pending().unwrap().is_empty());
    }
}
