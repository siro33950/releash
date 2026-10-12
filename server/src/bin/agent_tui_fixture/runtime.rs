#![allow(
    dead_code,
    reason = "shared support is compiled independently by multiple integration test targets"
)]

use serde::{Deserialize, Serialize};
use std::{
    env,
    io::Write,
    net::TcpStream,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

pub(crate) const FIXTURE_PLAN_ENV: &str = "RELEASH_AGENT_TUI_FIXTURE_PLAN";
pub(crate) const LIFECYCLE_COMMAND_RESULT_PREFIX: &str =
    "releash-fixture-lifecycle-command-result:";
const SESSION_START_INPUT_PREFIX: &str = "releash-fixture-session-start:";
const RAW_HOOK_INPUT_PREFIX: &str = "releash-fixture-hook-json:";
const BRACKETED_PASTE_START: &str = "\x1b[200~";
const BRACKETED_PASTE_END: &str = "\x1b[201~";
pub const FIXTURE_SESSION_KEY: &str = "fixture-session";
pub const FIXTURE_ATTEMPT_KEY: &str = "fixture-attempt";
pub const FIXTURE_TRANSCRIPT_REF: &str = "provider://fixture/transcript";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FixtureLifecycleSignal {
    pub session_key: String,
    pub attempt_key: String,
    pub transcript_ref: Option<String>,
    pub event: String,
    pub sequence: u64,
}

impl FixtureLifecycleSignal {
    pub fn new(event: &str, sequence: u64) -> Self {
        Self {
            session_key: FIXTURE_SESSION_KEY.to_string(),
            attempt_key: FIXTURE_ATTEMPT_KEY.to_string(),
            transcript_ref: Some(FIXTURE_TRANSCRIPT_REF.to_string()),
            event: event.to_string(),
            sequence,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FixtureLifecyclePayload {
    Signal { signal: FixtureLifecycleSignal },
    Raw { value: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FixtureLifecycleEmission {
    pub delay_before_ms: u64,
    pub payload: FixtureLifecyclePayload,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FixtureLifecycleCommand {
    pub executable: String,
    pub arguments: Vec<String>,
    pub environment: Vec<(String, String)>,
}

impl FixtureLifecycleEmission {
    pub fn signal(event: &str, sequence: u64) -> Self {
        Self {
            delay_before_ms: 0,
            payload: FixtureLifecyclePayload::Signal {
                signal: FixtureLifecycleSignal::new(event, sequence),
            },
        }
    }

    pub fn delayed_signal(event: &str, sequence: u64, delay_before_ms: u64) -> Self {
        Self {
            delay_before_ms,
            payload: FixtureLifecyclePayload::Signal {
                signal: FixtureLifecycleSignal::new(event, sequence),
            },
        }
    }

    pub fn raw(value: &str) -> Self {
        Self {
            delay_before_ms: 0,
            payload: FixtureLifecyclePayload::Raw {
                value: value.to_string(),
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FixturePlan {
    pub label: String,
    pub input_lines: usize,
    pub alternate_screen: bool,
    pub cursor_after_input: Option<(usize, u16, u16)>,
    pub report_terminal_size: bool,
    #[serde(default)]
    pub emit_input_completion_marker: bool,
    pub(crate) lifecycle_endpoint: String,
    pub lifecycle_command: Option<FixtureLifecycleCommand>,
    pub lifecycle: Vec<FixtureLifecycleEmission>,
    pub exit_code: u8,
}

impl FixturePlan {
    pub fn new(label: &str, lifecycle: Vec<FixtureLifecycleEmission>) -> Self {
        Self {
            label: label.to_string(),
            input_lines: 1,
            alternate_screen: true,
            cursor_after_input: None,
            report_terminal_size: false,
            emit_input_completion_marker: false,
            lifecycle_endpoint: String::new(),
            lifecycle_command: None,
            lifecycle,
            exit_code: 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FixtureLifecycleCommandResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

fn read_fixture_submission() -> String {
    let stdin = std::io::stdin();
    let mut submission = String::new();
    if stdin
        .read_line(&mut submission)
        .expect("read fixture PTY input")
        == 0
    {
        panic!("fixture PTY input ended before submission");
    }
    while submission.starts_with(BRACKETED_PASTE_START)
        && !submission
            .trim_end_matches(['\r', '\n'])
            .ends_with(BRACKETED_PASTE_END)
    {
        let mut continuation = String::new();
        if stdin
            .read_line(&mut continuation)
            .expect("read fixture PTY input continuation")
            == 0
        {
            panic!("fixture PTY input ended inside bracketed paste");
        }
        submission.push_str(&continuation);
    }
    submission.trim_end_matches(['\r', '\n']).to_string()
}

pub fn run_process() {
    let plan_json = env::var(FIXTURE_PLAN_ENV).expect("Agent TUI fixture plan is required");
    let plan: FixturePlan = serde_json::from_str(&plan_json).expect("parse Agent TUI fixture plan");

    if plan.alternate_screen {
        print!("\x1b[?1049h");
    }
    print!("\x1b[2J\x1b[H{} 日本語🙂\r\n", plan.label);
    std::io::stdout().flush().expect("flush fixture header");

    for index in 0..plan.input_lines {
        let input = read_fixture_submission();
        let input = input
            .strip_prefix(BRACKETED_PASTE_START)
            .and_then(|input| input.strip_suffix(BRACKETED_PASTE_END))
            .unwrap_or(&input);
        let displayed_input = input.replace('\r', "\\r").replace('\n', "\\n");
        print!("\x1b[1;32mreceived-{index}:{displayed_input}\x1b[0m\r\n");
        if plan.emit_input_completion_marker {
            print!("releash-fixture-input-complete-{index}\r\n");
        }
        let lifecycle_payload = input
            .strip_prefix(SESSION_START_INPUT_PREFIX)
            .map(|provider_session_id| FixtureLifecyclePayload::Raw {
                value: serde_json::json!({
                    "session_id": provider_session_id,
                    "transcript_path": format!("provider://fixture/{provider_session_id}"),
                    "hook_event_name": "SessionStart",
                })
                .to_string(),
            })
            .or_else(|| {
                input.strip_prefix(RAW_HOOK_INPUT_PREFIX).map(|value| {
                    FixtureLifecyclePayload::Raw {
                        value: value.to_string(),
                    }
                })
            });
        if let (Some(command), Some(payload)) = (plan.lifecycle_command.as_ref(), lifecycle_payload)
        {
            run_lifecycle_command(command, &payload);
        }
        if let Some((target_input, row, col)) = plan.cursor_after_input {
            if target_input == index {
                print!("\x1b[{row};{col}H");
            }
        }
        std::io::stdout().flush().expect("flush fixture response");
    }

    if plan.report_terminal_size {
        let (rows, cols) = current_terminal_size().expect("read fixture terminal size");
        print!("terminal-size:{rows}x{cols}\r\n");
        std::io::stdout()
            .flush()
            .expect("flush fixture terminal size");
    }

    for emission in &plan.lifecycle {
        thread::sleep(Duration::from_millis(emission.delay_before_ms));
        match &plan.lifecycle_command {
            Some(command) => run_lifecycle_command(command, &emission.payload),
            None => send_fixture_payload(&plan.lifecycle_endpoint, &emission.payload),
        }
    }

    if plan.lifecycle_command.is_some() {
        print!("provider-alive-after-lifecycle\r\n");
    }

    if plan.alternate_screen {
        print!("\x1b[?1049l");
    }
    std::io::stdout().flush().expect("flush fixture footer");
    if plan.exit_code != 0 {
        std::process::exit(i32::from(plan.exit_code));
    }
}

#[cfg(unix)]
fn current_terminal_size() -> Option<(u16, u16)> {
    let mut size = std::mem::MaybeUninit::<libc::winsize>::zeroed();
    let result = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, size.as_mut_ptr()) };
    if result == 0 {
        let size = unsafe { size.assume_init() };
        Some((size.ws_row, size.ws_col))
    } else {
        None
    }
}

#[cfg(not(unix))]
fn current_terminal_size() -> Option<(u16, u16)> {
    None
}

fn send_fixture_payload(endpoint: &str, payload: &FixtureLifecyclePayload) {
    let mut stream = TcpStream::connect(endpoint).expect("connect lifecycle fixture transport");
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .expect("set lifecycle write timeout");
    match payload {
        FixtureLifecyclePayload::Signal { signal } => {
            serde_json::to_writer(&mut stream, signal).expect("serialize lifecycle fixture signal");
        }
        FixtureLifecyclePayload::Raw { value } => stream
            .write_all(value.as_bytes())
            .expect("write raw lifecycle fixture payload"),
    }
    stream
        .write_all(b"\n")
        .expect("terminate lifecycle fixture payload");
    stream.flush().expect("flush lifecycle fixture payload");
}

fn run_lifecycle_command(command: &FixtureLifecycleCommand, payload: &FixtureLifecyclePayload) {
    let mut child = Command::new(&command.executable)
        .args(&command.arguments)
        .envs(command.environment.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn product lifecycle command");
    let stdin = child.stdin.as_mut().expect("open lifecycle command stdin");
    match payload {
        FixtureLifecyclePayload::Signal { signal } => {
            serde_json::to_writer(&mut *stdin, signal).expect("serialize lifecycle fixture signal");
        }
        FixtureLifecyclePayload::Raw { value } => stdin
            .write_all(value.as_bytes())
            .expect("write raw product lifecycle payload"),
    }
    stdin
        .write_all(b"\n")
        .expect("terminate product lifecycle payload");
    drop(child.stdin.take());
    let output = child
        .wait_with_output()
        .expect("wait for product lifecycle command");
    assert!(
        output.status.success(),
        "product lifecycle command failed: status={:?}, stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .expect("product lifecycle command stdout must be valid JSON");
    let result = FixtureLifecycleCommandResult {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    };
    println!(
        "{LIFECYCLE_COMMAND_RESULT_PREFIX}{}",
        serde_json::to_string(&result).expect("serialize lifecycle command result")
    );
}
