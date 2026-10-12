#![allow(
    dead_code,
    reason = "shared support is compiled independently by multiple integration test targets"
)]

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
pub use releashd::test_support::agent_tui_fixture::*;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

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
    pub lifecycle_commands: Vec<FixtureLifecycleCommandResult>,
}

pub struct FixtureRunOptions {
    pub input_lines: Vec<String>,
    pub resize_to: Option<PtySize>,
}

pub struct FixtureProcessLaunch {
    pub executable: String,
    pub arguments: Vec<String>,
    pub environment: Vec<(String, String)>,
}

pub fn fixture_process_launch(plan: &FixturePlan) -> FixtureProcessLaunch {
    FixtureProcessLaunch {
        executable: env!("CARGO_BIN_EXE_releash-agent-tui-fixture").to_string(),
        arguments: vec![],
        environment: vec![(
            FIXTURE_PLAN_ENV.to_string(),
            serde_json::to_string(plan).expect("serialize Agent TUI fixture plan"),
        )],
    }
}

impl Default for FixtureRunOptions {
    fn default() -> Self {
        Self {
            input_lines: vec!["operator-input".to_string()],
            resize_to: None,
        }
    }
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

    let expected_lifecycle_count = if plan.lifecycle_command.is_some() {
        0
    } else {
        plan.lifecycle.len()
    };
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
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_releash-agent-tui-fixture"));
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

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut timed_out = false;
    let exit_code = loop {
        if let Some(status) = child.try_wait().expect("poll Agent TUI fixture") {
            break status.exit_code();
        }
        if Instant::now() >= deadline {
            child.kill().expect("kill timed out Agent TUI fixture");
            child.wait().expect("reap timed out Agent TUI fixture");
            timed_out = true;
            break u32::MAX;
        }
        thread::sleep(Duration::from_millis(10));
    };
    drop(writer);

    let terminal_bytes = output_thread.join().expect("join Agent TUI PTY reader");
    let lifecycle = capture_thread
        .join()
        .expect("join Agent TUI lifecycle capture");

    let terminal_output = String::from_utf8_lossy(&terminal_bytes).into_owned();
    assert!(
        !timed_out,
        "Agent TUI fixture did not exit within fifteen seconds: {terminal_output}"
    );
    let lifecycle_commands = terminal_output
        .lines()
        .filter_map(|line| {
            line.find(LIFECYCLE_COMMAND_RESULT_PREFIX).map(|start| {
                let encoded = &line[start + LIFECYCLE_COMMAND_RESULT_PREFIX.len()..];
                serde_json::from_str(encoded.trim_end_matches('\r'))
                    .expect("decode lifecycle command result")
            })
        })
        .collect();

    FixtureRun {
        exit_code,
        terminal_output,
        lifecycle,
        lifecycle_commands,
    }
}

pub fn fixture_process_shell_command(plan: &FixturePlan) -> String {
    assert!(
        plan.lifecycle.is_empty(),
        "shell-launched fixture requires an independently prepared lifecycle endpoint"
    );
    let plan_json = serde_json::to_string(plan).expect("serialize Agent TUI fixture plan");
    let executable = env!("CARGO_BIN_EXE_releash-agent-tui-fixture");
    format!(
        "{FIXTURE_PLAN_ENV}={} {}",
        shell_quote(&plan_json),
        shell_quote(executable),
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
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
