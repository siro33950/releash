use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::{Deserialize, Serialize};
use std::env;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

const FIXTURE_PLAN_ENV: &str = "RELEASH_AGENT_TUI_FIXTURE_PLAN";
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
    pub report_terminal_size: bool,
    pub(crate) lifecycle_endpoint: String,
    pub lifecycle: Vec<FixtureLifecycleEmission>,
    pub exit_code: u8,
}

impl FixturePlan {
    pub fn new(label: &str, lifecycle: Vec<FixtureLifecycleEmission>) -> Self {
        Self {
            label: label.to_string(),
            input_lines: 1,
            alternate_screen: true,
            report_terminal_size: false,
            lifecycle_endpoint: String::new(),
            lifecycle,
            exit_code: 0,
        }
    }
}

#[derive(Debug)]
pub enum CapturedLifecyclePayload {
    Signal(FixtureLifecycleSignal),
    Invalid(String),
}

#[derive(Debug)]
pub struct CapturedLifecycleFrame {
    pub received_after: Duration,
    pub payload: CapturedLifecyclePayload,
}

pub struct FixtureRun {
    pub exit_code: u32,
    pub terminal_output: String,
    pub lifecycle: Vec<CapturedLifecycleFrame>,
}

pub struct FixtureRunOptions {
    pub input_lines: Vec<String>,
    pub resize_to: Option<PtySize>,
}

impl Default for FixtureRunOptions {
    fn default() -> Self {
        Self {
            input_lines: vec!["operator-input".to_string()],
            resize_to: None,
        }
    }
}

#[test]
fn agent_tui_fixture_process() {
    let Ok(plan_json) = env::var(FIXTURE_PLAN_ENV) else {
        return;
    };
    let plan: FixturePlan = serde_json::from_str(&plan_json).expect("parse Agent TUI fixture plan");

    if plan.alternate_screen {
        print!("\x1b[?1049h");
    }
    print!("\x1b[2J\x1b[H{} 日本語🙂\r\n", plan.label);
    std::io::stdout().flush().expect("flush fixture header");

    for index in 0..plan.input_lines {
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .expect("read fixture PTY input");
        print!("\x1b[1;32mreceived-{index}:{}\x1b[0m\r\n", input.trim());
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
        send_fixture_payload(&plan.lifecycle_endpoint, &emission.payload);
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

pub fn run_fixture(mut plan: FixturePlan, options: FixtureRunOptions) -> FixtureRun {
    assert_eq!(plan.input_lines, options.input_lines.len());
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind lifecycle fixture transport");
    listener
        .set_nonblocking(true)
        .expect("set lifecycle listener nonblocking");
    plan.lifecycle_endpoint = listener
        .local_addr()
        .expect("read lifecycle fixture address")
        .to_string();

    let expected_lifecycle_count = plan.lifecycle.len();
    let started_at = Instant::now();
    let capture_thread = thread::spawn(move || {
        collect_fixture_lifecycle(listener, expected_lifecycle_count, started_at)
    });

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open real PTY for Agent TUI fixture");
    let mut command = CommandBuilder::new(env::current_exe().expect("resolve test executable"));
    command.arg("--exact");
    command.arg(fixture_process_test_name());
    command.arg("--nocapture");
    command.arg("--test-threads=1");
    command.env(
        FIXTURE_PLAN_ENV,
        serde_json::to_string(&plan).expect("serialize Agent TUI fixture plan"),
    );

    let mut child = pair
        .slave
        .spawn_command(command)
        .expect("spawn Agent TUI fixture in real PTY");
    drop(pair.slave);

    if let Some(size) = options.resize_to {
        pair.master
            .resize(size)
            .expect("resize Agent TUI fixture PTY");
    }

    let mut writer = pair
        .master
        .take_writer()
        .expect("take Agent TUI fixture PTY writer");
    let mut reader = pair
        .master
        .try_clone_reader()
        .expect("clone Agent TUI fixture PTY reader");
    let output_thread = thread::spawn(move || read_pty_to_end(&mut reader));

    for input in &options.input_lines {
        writer
            .write_all(format!("{input}\r").as_bytes())
            .expect("write Agent TUI fixture PTY input");
        writer.flush().expect("flush Agent TUI fixture PTY input");
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    let exit_code = loop {
        if let Some(status) = child.try_wait().expect("poll Agent TUI fixture") {
            break status.exit_code();
        }
        if Instant::now() >= deadline {
            child.kill().expect("kill timed out Agent TUI fixture");
            panic!("Agent TUI fixture did not exit within five seconds");
        }
        thread::sleep(Duration::from_millis(10));
    };
    drop(writer);

    let terminal_bytes = output_thread.join().expect("join Agent TUI PTY reader");
    let lifecycle = capture_thread
        .join()
        .expect("join Agent TUI lifecycle capture");

    FixtureRun {
        exit_code,
        terminal_output: String::from_utf8_lossy(&terminal_bytes).into_owned(),
        lifecycle,
    }
}

pub fn fixture_process_shell_command(plan: &FixturePlan) -> String {
    assert!(
        plan.lifecycle.is_empty(),
        "shell-launched fixture requires an independently prepared lifecycle endpoint"
    );
    let plan_json = serde_json::to_string(plan).expect("serialize Agent TUI fixture plan");
    let executable = env::current_exe().expect("resolve test executable");
    format!(
        "{FIXTURE_PLAN_ENV}={} {} --exact {} --nocapture --test-threads=1",
        shell_quote(&plan_json),
        shell_quote(&executable.to_string_lossy()),
        shell_quote(&fixture_process_test_name()),
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn fixture_process_test_name() -> String {
    let module = module_path!();
    let module_without_crate = module.split_once("::").map_or(module, |(_, rest)| rest);
    format!("{module_without_crate}::agent_tui_fixture_process")
}

fn read_pty_to_end(reader: &mut impl Read) -> Vec<u8> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => output.extend_from_slice(&buffer[..read]),
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) if error.raw_os_error() == Some(5) => break,
            Err(error) => panic!("read Agent TUI fixture PTY: {error}"),
        }
    }
    output
}

fn collect_fixture_lifecycle(
    listener: TcpListener,
    expected_count: usize,
    started_at: Instant,
) -> Vec<CapturedLifecycleFrame> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut captured = Vec::new();
    while captured.len() < expected_count {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream
                    .set_nonblocking(false)
                    .expect("set lifecycle fixture stream blocking");
                let mut payload = String::new();
                stream
                    .read_to_string(&mut payload)
                    .expect("read lifecycle fixture payload");
                let payload = payload.trim_end().to_string();
                let parsed = match serde_json::from_str(&payload) {
                    Ok(signal) => CapturedLifecyclePayload::Signal(signal),
                    Err(_) => CapturedLifecyclePayload::Invalid(payload),
                };
                captured.push(CapturedLifecycleFrame {
                    received_after: started_at.elapsed(),
                    payload: parsed,
                });
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("accept lifecycle fixture payload: {error}"),
        }
    }
    captured
}

pub fn parsed_signals(run: &FixtureRun) -> Vec<&FixtureLifecycleSignal> {
    run.lifecycle
        .iter()
        .filter_map(|frame| match &frame.payload {
            CapturedLifecyclePayload::Signal(signal) => Some(signal),
            CapturedLifecyclePayload::Invalid(_) => None,
        })
        .collect()
}

pub fn signal_events(run: &FixtureRun) -> Vec<&str> {
    parsed_signals(run)
        .into_iter()
        .map(|signal| signal.event.as_str())
        .collect()
}
