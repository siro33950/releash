use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::time::{SystemTime, UNIX_EPOCH};

const LOG_DIRECTORY_NAME: &str = "logs";
const ACTIVE_FILE_NAME: &str = "releash.log";
const LOCK_FILE_NAME: &str = "releash.lock";
const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_FILE_COUNT: usize = 5;
const QUEUE_CAPACITY: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalLogProcess {
    Gui,
    Cli,
}

impl LocalLogProcess {
    fn as_str(self) -> &'static str {
        match self {
            Self::Gui => "gui",
            Self::Cli => "cli",
        }
    }
}

#[derive(Debug)]
pub(crate) enum LocalLogInitError {
    Io(io::Error),
    AlreadyInitialized,
}

impl std::fmt::Display for LocalLogInitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "local log initialization failed: {error}"),
            Self::AlreadyInitialized => formatter.write_str("local logger is already initialized"),
        }
    }
}

impl std::error::Error for LocalLogInitError {}

impl From<io::Error> for LocalLogInitError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub(crate) fn init(data_dir: &Path, process: LocalLogProcess) -> Result<(), LocalLogInitError> {
    init_with_limits(data_dir, process, MAX_FILE_BYTES, MAX_FILE_COUNT)
}

fn init_with_limits(
    data_dir: &Path,
    process: LocalLogProcess,
    max_file_bytes: u64,
    max_file_count: usize,
) -> Result<(), LocalLogInitError> {
    let logger = LocalFileLogger::new(data_dir, process, max_file_bytes, max_file_count)?;
    let logger = Box::leak(Box::new(logger));
    log::set_logger(logger).map_err(|_| LocalLogInitError::AlreadyInitialized)?;
    log::set_max_level(log::LevelFilter::Warn);
    Ok(())
}

struct PendingLogRecord {
    timestamp_unix_ms: u64,
    level: log::Level,
    target: String,
    message: String,
    dropped_records_before: u64,
}

impl PendingLogRecord {
    fn new(level: log::Level, target: &str, message: String) -> Self {
        Self {
            timestamp_unix_ms: timestamp_unix_ms(),
            level,
            target: target.to_string(),
            message,
            dropped_records_before: 0,
        }
    }

    fn dropped_notice(dropped_records: u64) -> Self {
        Self {
            timestamp_unix_ms: timestamp_unix_ms(),
            level: log::Level::Warn,
            target: module_path!().to_string(),
            message: format!("Local log records dropped: count={dropped_records}"),
            dropped_records_before: dropped_records,
        }
    }
}

fn timestamp_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

enum WriterCommand {
    Record(PendingLogRecord),
    Flush(SyncSender<()>),
}

struct LocalFileLogger {
    sender: SyncSender<WriterCommand>,
    dropped_records: AtomicU64,
}

impl LocalFileLogger {
    fn new(
        data_dir: &Path,
        process: LocalLogProcess,
        max_file_bytes: u64,
        max_file_count: usize,
    ) -> Result<Self, io::Error> {
        Self::new_with_queue_capacity(
            data_dir,
            process,
            max_file_bytes,
            max_file_count,
            QUEUE_CAPACITY,
        )
    }

    fn new_with_queue_capacity(
        data_dir: &Path,
        process: LocalLogProcess,
        max_file_bytes: u64,
        max_file_count: usize,
        queue_capacity: usize,
    ) -> Result<Self, io::Error> {
        let (sender, receiver) = mpsc::sync_channel(queue_capacity);
        let writer = LocalLogWriter::new(data_dir, process, max_file_bytes, max_file_count);
        std::thread::Builder::new()
            .name("releash-local-log-writer".to_string())
            .spawn(move || writer.run(receiver))?;
        Ok(Self {
            sender,
            dropped_records: AtomicU64::new(0),
        })
    }

    fn enqueue(&self, mut record: PendingLogRecord) {
        record.dropped_records_before = self.dropped_records.swap(0, Ordering::Relaxed);
        match self.sender.try_send(WriterCommand::Record(record)) {
            Ok(()) => {}
            Err(TrySendError::Full(WriterCommand::Record(record)))
            | Err(TrySendError::Disconnected(WriterCommand::Record(record))) => {
                self.add_dropped(record.dropped_records_before.saturating_add(1));
            }
            Err(TrySendError::Full(WriterCommand::Flush(_)))
            | Err(TrySendError::Disconnected(WriterCommand::Flush(_))) => unreachable!(),
        }
    }

    fn add_dropped(&self, count: u64) {
        let _ =
            self.dropped_records
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                    Some(current.saturating_add(count))
                });
    }

    fn drain_dropped_notice(&self) -> bool {
        let dropped_records = self.dropped_records.swap(0, Ordering::Relaxed);
        if dropped_records == 0 {
            return true;
        }
        if self
            .sender
            .send(WriterCommand::Record(PendingLogRecord::dropped_notice(
                dropped_records,
            )))
            .is_err()
        {
            self.add_dropped(dropped_records);
            return false;
        }
        true
    }
}

impl log::Log for LocalFileLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            self.enqueue(PendingLogRecord::new(
                record.level(),
                record.target(),
                record.args().to_string(),
            ));
        }
    }

    fn flush(&self) {
        if !self.drain_dropped_notice() {
            return;
        }
        let (completed, completion) = mpsc::sync_channel(0);
        if self.sender.send(WriterCommand::Flush(completed)).is_ok() {
            let _ = completion.recv();
        }
    }
}

struct LocalLogWriter {
    directory: PathBuf,
    directory_initialized: bool,
    process: LocalLogProcess,
    max_file_bytes: u64,
    max_file_count: usize,
}

impl LocalLogWriter {
    fn new(
        data_dir: &Path,
        process: LocalLogProcess,
        max_file_bytes: u64,
        max_file_count: usize,
    ) -> Self {
        Self {
            directory: data_dir.join(LOG_DIRECTORY_NAME),
            directory_initialized: false,
            process,
            max_file_bytes,
            max_file_count: max_file_count.max(1),
        }
    }

    fn run(mut self, receiver: Receiver<WriterCommand>) {
        while let Ok(command) = receiver.recv() {
            match command {
                WriterCommand::Record(record) => {
                    let _ = self.append(&record);
                }
                WriterCommand::Flush(completed) => {
                    let _ = completed.send(());
                }
            }
        }
    }

    fn active_path(&self) -> PathBuf {
        self.directory.join(ACTIVE_FILE_NAME)
    }

    fn generation_path(&self, generation: usize) -> PathBuf {
        self.directory.join(format!("releash.{generation}.log"))
    }

    fn lock_path(&self) -> PathBuf {
        self.directory.join(LOCK_FILE_NAME)
    }

    fn append(&mut self, record: &PendingLogRecord) -> Result<(), io::Error> {
        let record = self.serialize(record)?;
        if !self.directory_initialized {
            fs::create_dir_all(&self.directory)?;
            self.directory_initialized = true;
        }
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(self.lock_path())?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let write_result = self.append_locked(&record);
        let unlock_result = fs2::FileExt::unlock(&lock);
        write_result.and(unlock_result)
    }

    fn serialize(&self, record: &PendingLogRecord) -> Result<Vec<u8>, io::Error> {
        let mut serialized = serde_json::json!({
            "timestamp_unix_ms": record.timestamp_unix_ms,
            "level": record.level.as_str(),
            "target": record.target,
            "message": record.message,
            "process": self.process.as_str(),
        });
        if record.dropped_records_before > 0 {
            serialized["dropped_records_before"] =
                serde_json::Value::from(record.dropped_records_before);
        }
        let mut serialized = serde_json::to_vec(&serialized).map_err(io::Error::other)?;
        serialized.push(b'\n');
        Ok(serialized)
    }

    fn append_locked(&self, record: &[u8]) -> Result<(), io::Error> {
        let active_path = self.active_path();
        if self.max_file_count == 1
            && fs::metadata(&active_path).is_ok_and(|metadata| metadata.len() > self.max_file_bytes)
        {
            fs::remove_file(&active_path)?;
        }

        let mut active = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&active_path)?;
        active.write_all(record)?;
        active.flush()?;
        let active_length = active.metadata()?.len();
        drop(active);

        if self.max_file_count > 1 && active_length > self.max_file_bytes {
            self.rotate_locked()?;
        }
        Ok(())
    }

    fn rotate_locked(&self) -> Result<(), io::Error> {
        let active_path = self.active_path();
        for generation in (1..self.max_file_count).rev() {
            let source = if generation == 1 {
                active_path.clone()
            } else {
                self.generation_path(generation - 1)
            };
            let destination = self.generation_path(generation);
            match fs::remove_file(&destination) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            match fs::rename(source, destination) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "local_log_test.rs"]
mod local_log_tests;
