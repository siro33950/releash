use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::domain::agent_session::gateway::AgentRuntimeEvent;

use super::claude::convert::{convert_claude_message, ClaudeConversion, ClaudeConvertState};
use super::claude::wire::ClaudeWireMode;
use super::codex::{convert_jsonrpc_message, CodexConvertState};

mod snapshot;

const FIXTURE_ROOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/infrastructure/agent_session/fixtures"
);
const UPDATE_GOLDEN_ENV: &str = "UPDATE_GOLDEN";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FixtureBackend {
    Claude,
    Codex,
}

impl FixtureBackend {
    fn directory_name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

#[derive(Debug)]
pub(crate) struct ReplayedFixture {
    pub backend: FixtureBackend,
    pub name: String,
    pub events: Vec<AgentRuntimeEvent>,
    directory: PathBuf,
    convert_output: String,
}

impl ReplayedFixture {
    pub(crate) fn convert_golden_path(&self) -> PathBuf {
        self.directory.join("convert.golden")
    }

    pub(crate) fn read_model_golden_path(&self) -> PathBuf {
        self.directory.join("read_model.golden")
    }

    fn assert_convert_golden(&self) {
        assert_golden(&self.convert_golden_path(), &self.convert_output);
    }
}

#[derive(Serialize)]
struct ReplayLine {
    line_index: usize,
    events: Vec<snapshot::RuntimeEventSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_responses: Option<Vec<Value>>,
}

pub(crate) fn replay_backend(backend: FixtureBackend) -> Vec<ReplayedFixture> {
    let directories = fixture_directories(backend);
    assert!(
        !directories.is_empty(),
        "no {} wire fixtures found under {FIXTURE_ROOT}",
        backend.directory_name()
    );
    directories
        .into_iter()
        .map(|directory| replay_fixture(backend, directory))
        .collect()
}

fn fixture_directories(backend: FixtureBackend) -> Vec<PathBuf> {
    let backend_root = Path::new(FIXTURE_ROOT).join(backend.directory_name());
    let mut directories = fs::read_dir(&backend_root)
        .unwrap_or_else(|error| {
            panic!(
                "failed to read fixture directory {}: {error}",
                backend_root.display()
            )
        })
        .map(|entry| entry.expect("failed to read fixture entry").path())
        .filter(|path| path.is_dir() && path.join("wire.jsonl").is_file())
        .collect::<Vec<_>>();
    directories.sort();
    directories
}

fn replay_fixture(backend: FixtureBackend, directory: PathBuf) -> ReplayedFixture {
    let wire_path = directory.join("wire.jsonl");
    let wire = fs::read_to_string(&wire_path)
        .unwrap_or_else(|error| panic!("failed to read fixture {}: {error}", wire_path.display()));
    assert!(!wire.is_empty(), "fixture {} is empty", wire_path.display());

    let mut events = Vec::new();
    let lines = match backend {
        FixtureBackend::Claude => replay_claude_lines(&wire_path, &wire, &mut events),
        FixtureBackend::Codex => replay_codex_lines(&wire_path, &wire, &mut events),
    };
    let name = directory
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("fixture")
        .to_string();
    ReplayedFixture {
        backend,
        name,
        events,
        directory,
        convert_output: pretty_json(&lines),
    }
}

fn replay_claude_lines(
    wire_path: &Path,
    wire: &str,
    all_events: &mut Vec<AgentRuntimeEvent>,
) -> Vec<ReplayLine> {
    let mut state = ClaudeConvertState::new(None, ClaudeWireMode::AcceptEdits);
    wire.lines()
        .enumerate()
        .map(|(index, raw_line)| {
            let message = parse_fixture_line(wire_path, index, raw_line);
            let ClaudeConversion {
                events,
                auto_responses,
            } = convert_claude_message(&message, &mut state);
            let event_snapshots = events.iter().map(Into::into).collect();
            all_events.extend(events.iter().cloned());
            ReplayLine {
                line_index: index + 1,
                events: event_snapshots,
                auto_responses: Some(auto_responses),
            }
        })
        .collect()
}

fn replay_codex_lines(
    wire_path: &Path,
    wire: &str,
    all_events: &mut Vec<AgentRuntimeEvent>,
) -> Vec<ReplayLine> {
    let mut state = CodexConvertState::default();
    wire.lines()
        .enumerate()
        .map(|(index, raw_line)| {
            let message = parse_fixture_line(wire_path, index, raw_line);
            let events = convert_jsonrpc_message(&message, &mut state);
            let event_snapshots = events.iter().map(Into::into).collect();
            all_events.extend(events.iter().cloned());
            ReplayLine {
                line_index: index + 1,
                events: event_snapshots,
                auto_responses: None,
            }
        })
        .collect()
}

fn parse_fixture_line(wire_path: &Path, index: usize, raw_line: &str) -> Value {
    serde_json::from_str(raw_line).unwrap_or_else(|error| {
        panic!(
            "invalid JSONL fixture {} line {}: {error}",
            wire_path.display(),
            index + 1
        )
    })
}

pub(crate) fn pretty_json<T: Serialize>(value: &T) -> String {
    let mut output = serde_json::to_string_pretty(value).expect("golden value must serialize");
    output.push('\n');
    output
}

pub(crate) fn assert_golden(path: &Path, actual: &str) {
    let update = update_golden_enabled(std::env::var_os(UPDATE_GOLDEN_ENV).as_deref());
    assert_golden_with_update(path, actual, update);
}

fn update_golden_enabled(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}

fn assert_golden_with_update(path: &Path, actual: &str, update: bool) {
    if update {
        fs::write(path, actual)
            .unwrap_or_else(|error| panic!("failed to update golden {}: {error}", path.display()));
        return;
    }
    let expected = fs::read_to_string(path).unwrap_or_else(|error| {
        panic!(
            "failed to read golden {}: {error}; run with UPDATE_GOLDEN=1 to generate it",
            path.display()
        )
    });
    let expected = expected.replace("\r\n", "\n");
    if expected != actual {
        panic!(
            "golden mismatch for {}\n{}\nrun with UPDATE_GOLDEN=1 after reviewing the change",
            path.display(),
            first_difference(&expected, actual)
        );
    }
}

fn first_difference(expected: &str, actual: &str) -> String {
    let expected_lines = expected.lines().collect::<Vec<_>>();
    let actual_lines = actual.lines().collect::<Vec<_>>();
    let line_index = (0..expected_lines.len().max(actual_lines.len()))
        .find(|index| expected_lines.get(*index) != actual_lines.get(*index))
        .unwrap_or(0);
    format!(
        "first difference at line {}:\nexpected: {}\nactual:   {}",
        line_index + 1,
        expected_lines.get(line_index).copied().unwrap_or("<EOF>"),
        actual_lines.get(line_index).copied().unwrap_or("<EOF>")
    )
}

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::panic::AssertUnwindSafe;

    use super::*;

    #[test]
    fn claude_fixtures_match_convert_golden() {
        for fixture in replay_backend(FixtureBackend::Claude) {
            fixture.assert_convert_golden();
        }
    }

    #[test]
    fn codex_fixtures_match_convert_golden() {
        for fixture in replay_backend(FixtureBackend::Codex) {
            fixture.assert_convert_golden();
        }
    }

    #[test]
    fn golden_helper_accepts_matching_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.golden");
        fs::write(&path, "same\n").unwrap();

        assert_golden_with_update(&path, "same\n", false);
    }

    #[test]
    fn golden_helper_accepts_crlf_expected_for_lf_actual() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.golden");
        fs::write(&path, "first\r\nsecond\r\n").unwrap();

        assert_golden_with_update(&path, "first\nsecond\n", false);
    }

    #[test]
    fn golden_helper_reports_mismatch_and_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.golden");
        fs::write(&path, "before\n").unwrap();

        let mismatch = panic_payload(std::panic::catch_unwind(AssertUnwindSafe(|| {
            assert_golden_with_update(&path, "after\n", false);
        })));
        let missing = panic_payload(std::panic::catch_unwind(AssertUnwindSafe(|| {
            assert_golden_with_update(&dir.path().join("missing.golden"), "value\n", false);
        })));

        assert!(mismatch.contains(&path.display().to_string()));
        assert!(mismatch.contains("first difference at line 1:"));
        assert!(mismatch.contains("expected: before"));
        assert!(mismatch.contains("actual:   after"));
        assert!(missing.contains("run with UPDATE_GOLDEN=1 to generate it"));
    }

    fn panic_payload(result: Result<(), Box<dyn Any + Send>>) -> String {
        let payload = result.expect_err("golden assertion should panic");
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                payload
                    .downcast_ref::<&str>()
                    .map(|message| (*message).to_string())
            })
            .expect("panic payload should be a string")
    }

    #[test]
    fn update_golden_gate_accepts_only_one() {
        assert_eq!(UPDATE_GOLDEN_ENV, "UPDATE_GOLDEN");
        assert!(update_golden_enabled(Some(OsStr::new("1"))));
        assert!(!update_golden_enabled(None));
        for value in ["", "0", "true"] {
            assert!(!update_golden_enabled(Some(OsStr::new(value))));
        }
    }

    #[test]
    fn golden_helper_update_overwrites_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.golden");
        fs::write(&path, "before\n").unwrap();

        let update = update_golden_enabled(Some(OsStr::new("1")));
        assert_golden_with_update(&path, "after\n", update);

        assert_eq!(fs::read_to_string(path).unwrap(), "after\n");
    }
}
