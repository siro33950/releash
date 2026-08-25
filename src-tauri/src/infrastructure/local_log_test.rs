use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use opentelemetry::logs::AnyValue;
use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLogRecord, SdkLoggerProvider};

use super::*;

const CHILD_TEST_NAME: &str =
    "infrastructure::local_log::local_log_tests::test_local_log_child_writer";
const CONFIGURATION_CHILD_TEST_NAME: &str =
    "infrastructure::local_log::local_log_tests::test_local_log_file_only構成_child";
const CONFIGURATION_CHILD_SUCCESS_MARKER: &str = "releash-local-log-configuration-test-passed";
const TEST_MAX_FILE_BYTES: u64 = 256;

fn serialized_record_with_size(size: usize) -> Vec<u8> {
    let serialize = |message: &str| {
        let mut record = serde_json::to_vec(&serde_json::json!({
            "timestamp_unix_ms": 0,
            "level": "WARN",
            "target": "local_log_test",
            "message": message,
            "process": "gui",
        }))
        .unwrap();
        record.push(b'\n');
        record
    };
    let empty_record_size = serialize("").len();
    assert!(size >= empty_record_size);
    let record = serialize(&"x".repeat(size - empty_record_size));
    assert_eq!(record.len(), size);
    record
}

fn log_files(data_dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(data_dir.join(LOG_DIRECTORY_NAME))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name == ACTIVE_FILE_NAME
                        || (name.starts_with("releash.") && name.ends_with(".log"))
                })
        })
        .collect();
    files.sort();
    files
}

fn assert_log_file_count_bound(data_dir: &Path) -> Vec<PathBuf> {
    let files = log_files(data_dir);
    assert!(!files.is_empty());
    assert!(files.len() <= MAX_FILE_COUNT);
    files
}

fn records(data_dir: &Path) -> Vec<serde_json::Value> {
    log_files(data_dir)
        .into_iter()
        .flat_map(|path| {
            fs::read_to_string(path)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn writer(data_dir: &Path) -> LocalLogWriter {
    fs::create_dir_all(data_dir.join(LOG_DIRECTORY_NAME)).unwrap();
    LocalLogWriter::new(
        data_dir,
        LocalLogProcess::Gui,
        TEST_MAX_FILE_BYTES,
        MAX_FILE_COUNT,
    )
}

fn child_writer(
    data_dir: &Path,
    process: LocalLogProcess,
    writer: &str,
    count: usize,
    max_file_bytes: u64,
) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg(CHILD_TEST_NAME)
        .arg("--ignored")
        .env("RELEASH_LOCAL_LOG_TEST_DIR", data_dir)
        .env("RELEASH_LOCAL_LOG_TEST_PROCESS", process.as_str())
        .env("RELEASH_LOCAL_LOG_TEST_WRITER", writer)
        .env("RELEASH_LOCAL_LOG_TEST_COUNT", count.to_string())
        .env(
            "RELEASH_LOCAL_LOG_TEST_MAX_BYTES",
            max_file_bytes.to_string(),
        )
        .spawn()
        .unwrap()
}

fn exported_body(record: &SdkLogRecord) -> Option<String> {
    record.body().map(|value| match value {
        AnyValue::String(value) => value.to_string(),
        _ => format!("{value:?}"),
    })
}

#[test]
#[ignore]
fn test_local_log_child_writer() {
    let Some(data_dir) = std::env::var_os("RELEASH_LOCAL_LOG_TEST_DIR") else {
        return;
    };
    let process = match std::env::var("RELEASH_LOCAL_LOG_TEST_PROCESS")
        .unwrap()
        .as_str()
    {
        "gui" => LocalLogProcess::Gui,
        "cli" => LocalLogProcess::Cli,
        value => panic!("unexpected process kind: {value}"),
    };
    let writer = std::env::var("RELEASH_LOCAL_LOG_TEST_WRITER").unwrap();
    let count = std::env::var("RELEASH_LOCAL_LOG_TEST_COUNT")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let max_file_bytes = std::env::var("RELEASH_LOCAL_LOG_TEST_MAX_BYTES")
        .unwrap()
        .parse::<u64>()
        .unwrap();
    init_with_limits(
        Path::new(&data_dir),
        process,
        max_file_bytes,
        MAX_FILE_COUNT,
    )
    .unwrap();

    log::info!(target: "local_log_test", "writer={writer} info must not be written");
    for index in 0..count {
        log::warn!(
            target: "local_log_test",
            "writer={writer} index={index} payload=xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
        );
    }
    log::error!(target: "local_log_test", "writer={writer} terminal error");
    log::logger().flush();
}

#[test]
#[ignore]
fn test_local_log_file_only構成_child() {
    let Some(data_dir) = std::env::var_os("RELEASH_LOCAL_LOG_CONFIGURATION_TEST_DIR") else {
        return;
    };
    let exporter = InMemoryLogExporter::default();
    let provider = SdkLoggerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    crate::infrastructure::telemetry::crash::init_crash_reporting(
        Some(provider.clone()),
        true,
        true,
    );
    init_with_limits(
        Path::new(&data_dir),
        LocalLogProcess::Gui,
        1_024 * 1_024,
        MAX_FILE_COUNT,
    )
    .unwrap();

    log::warn!(target: "local_log_configuration_test", "local-file-only-marker");
    crate::infrastructure::telemetry::crash::report_error(
        "rust",
        "local-log-configuration-test",
        "crash-otlp-only-marker",
        None,
    );
    log::logger().flush();
    provider.force_flush().unwrap();

    let local_records = records(Path::new(&data_dir));
    assert!(local_records
        .iter()
        .any(|record| record["message"] == "local-file-only-marker"));
    assert!(!local_records
        .iter()
        .any(|record| record["message"] == "crash-otlp-only-marker"));
    let exported_bodies: Vec<String> = exporter
        .get_emitted_logs()
        .unwrap()
        .iter()
        .filter_map(|log| exported_body(&log.record))
        .collect();
    assert!(exported_bodies
        .iter()
        .any(|body| body == "crash-otlp-only-marker"));
    assert!(!exported_bodies
        .iter()
        .any(|body| body == "local-file-only-marker"));
    println!("{CONFIGURATION_CHILD_SUCCESS_MARKER}");
}

#[test]
fn test_local_log_file_only構成では通常logをfileだけへ送りcrash_logだけをotlpへ送る() {
    // Given
    let directory = tempfile::tempdir().unwrap();

    // When
    let output = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg(CONFIGURATION_CHILD_TEST_NAME)
        .arg("--ignored")
        .arg("--nocapture")
        .env("RELEASH_LOCAL_LOG_CONFIGURATION_TEST_DIR", directory.path())
        .output()
        .unwrap();

    // Then
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "configuration child failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout
            .lines()
            .any(|line| line == CONFIGURATION_CHILD_SUCCESS_MARKER),
        "configuration child success marker is missing\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn test_local_log_初期化とflushだけではdata_dirを作らない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let missing_data_dir = directory.path().join("missing");

    // When
    let logger = LocalFileLogger::new(
        &missing_data_dir,
        LocalLogProcess::Cli,
        TEST_MAX_FILE_BYTES,
        MAX_FILE_COUNT,
    )
    .unwrap();
    log::Log::flush(&logger);

    // Then
    assert!(!missing_data_dir.exists());
}

#[test]
fn test_local_log_process間lock保持中もlog呼び出しをブロックせず破棄件数を残す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let logs_directory = directory.path().join(LOG_DIRECTORY_NAME);
    fs::create_dir_all(&logs_directory).unwrap();
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(logs_directory.join(LOCK_FILE_NAME))
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    let logger = Arc::new(
        LocalFileLogger::new_with_queue_capacity(
            directory.path(),
            LocalLogProcess::Gui,
            1_024 * 1_024,
            MAX_FILE_COUNT,
            1,
        )
        .unwrap(),
    );
    let caller = Arc::clone(&logger);
    let (completed, completion) = mpsc::sync_channel(0);

    // When
    std::thread::spawn(move || {
        let record = log::Record::builder()
            .args(format_args!("writer lock is held"))
            .level(log::Level::Error)
            .target("local_log_test")
            .build();
        log::Log::log(caller.as_ref(), &record);
        completed.send(()).unwrap();
    });

    // Then
    completion
        .recv_timeout(Duration::from_secs(1))
        .expect("log caller must not wait for the process lock");
    for _ in 0..10_000 {
        let record = log::Record::builder()
            .args(format_args!("queue pressure"))
            .level(log::Level::Warn)
            .target("local_log_test")
            .build();
        log::Log::log(logger.as_ref(), &record);
    }
    assert!(logger.dropped_records.load(Ordering::Relaxed) > 0);

    fs2::FileExt::unlock(&lock).unwrap();
    log::Log::flush(logger.as_ref());
    assert!(records(directory.path()).iter().any(|record| {
        record["dropped_records_before"]
            .as_u64()
            .is_some_and(|count| count > 0)
    }));
}

#[test]
fn test_local_log_guiとcliのwarningとerrorをprocess終了後も共通fileから参照できる() {
    // Given
    let directory = tempfile::tempdir().unwrap();

    // When
    let mut gui = child_writer(
        directory.path(),
        LocalLogProcess::Gui,
        "gui",
        1,
        1_024 * 1_024,
    );
    assert!(gui.wait().unwrap().success());
    let mut cli = child_writer(
        directory.path(),
        LocalLogProcess::Cli,
        "cli",
        1,
        1_024 * 1_024,
    );
    assert!(cli.wait().unwrap().success());

    // Then
    let records = records(directory.path());
    assert_eq!(records.len(), 4);
    for record in &records {
        assert!(record["timestamp_unix_ms"].as_u64().is_some());
        assert!(matches!(record["level"].as_str(), Some("WARN" | "ERROR")));
        assert_eq!(record["target"], "local_log_test");
        assert!(record["message"].as_str().unwrap().contains("writer="));
        assert!(matches!(record["process"].as_str(), Some("gui" | "cli")));
    }
    assert!(records.iter().any(|record| record["process"] == "gui"));
    assert!(records.iter().any(|record| record["process"] == "cli"));
    assert!(!records.iter().any(|record| record["message"]
        .as_str()
        .unwrap()
        .contains("info must not")));
}

#[test]
fn test_local_log_guiと複数cliの同時rotationでrecordを混線させず最大5fileを保持する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let mut children = vec![
        child_writer(directory.path(), LocalLogProcess::Gui, "gui", 80, 512),
        child_writer(directory.path(), LocalLogProcess::Cli, "cli-1", 80, 512),
        child_writer(directory.path(), LocalLogProcess::Cli, "cli-2", 80, 512),
    ];

    // When
    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }

    // Then
    let files = assert_log_file_count_bound(directory.path());
    assert!(files.len() >= MAX_FILE_COUNT - 1);
    for record in records(directory.path()) {
        assert_eq!(record["target"], "local_log_test");
        let message = record["message"].as_str().unwrap();
        assert!(message.starts_with("writer="));
        assert!(message.contains(" index=") || message.ends_with(" terminal error"));
        assert!(matches!(record["process"].as_str(), Some("gui" | "cli")));
    }
}

#[test]
fn test_local_log_空activeへ上限ちょうどのrecordを書いてrotateしない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let writer = writer(directory.path());
    let record = serialized_record_with_size(usize::try_from(TEST_MAX_FILE_BYTES).unwrap());

    // When
    writer.append_locked(&record).unwrap();

    // Then
    let files = assert_log_file_count_bound(directory.path());
    assert_eq!(files, vec![writer.active_path()]);
    assert_eq!(fs::metadata(&files[0]).unwrap().len(), TEST_MAX_FILE_BYTES);
    assert_eq!(records(directory.path()).len(), 1);
}

#[test]
fn test_local_log_空activeへの上限超過recordを分割せず1つのjson行として保持する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let writer = writer(directory.path());
    let record_size = usize::try_from(TEST_MAX_FILE_BYTES).unwrap() * (MAX_FILE_COUNT + 2) + 1;
    let record = serialized_record_with_size(record_size);

    // When
    writer.append_locked(&record).unwrap();

    // Then
    let files = assert_log_file_count_bound(directory.path());
    assert_eq!(files, vec![writer.generation_path(1)]);
    assert_eq!(fs::metadata(&files[0]).unwrap().len(), record_size as u64);
    assert_eq!(records(directory.path()).len(), 1);
}

#[test]
fn test_local_log_既存record後の上限ちょうどのrecordを同じfileへ書いてからrotateする() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let writer = writer(directory.path());
    let existing_record = serialized_record_with_size(128);
    writer.append_locked(&existing_record).unwrap();
    let record = serialized_record_with_size(usize::try_from(TEST_MAX_FILE_BYTES).unwrap());

    // When
    writer.append_locked(&record).unwrap();

    // Then
    let files = assert_log_file_count_bound(directory.path());
    assert_eq!(files, vec![writer.generation_path(1)]);
    assert_eq!(
        fs::metadata(&files[0]).unwrap().len(),
        128 + TEST_MAX_FILE_BYTES
    );
    assert_eq!(records(directory.path()).len(), 2);
}

#[test]
fn test_local_log_既存record後の上限超過recordも分割せず世代上限を守る() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let writer = writer(directory.path());
    writer
        .append_locked(&serialized_record_with_size(128))
        .unwrap();
    let record_size = usize::try_from(TEST_MAX_FILE_BYTES).unwrap() * (MAX_FILE_COUNT + 2) + 1;
    let record = serialized_record_with_size(record_size);

    // When
    writer.append_locked(&record).unwrap();

    // Then
    let files = assert_log_file_count_bound(directory.path());
    assert_eq!(files, vec![writer.generation_path(1)]);
    assert_eq!(
        fs::metadata(&files[0]).unwrap().len(),
        128 + record_size as u64
    );
    assert_eq!(records(directory.path()).len(), 2);
}
