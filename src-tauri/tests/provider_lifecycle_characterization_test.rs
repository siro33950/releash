use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use releash_lib::provider_lifecycle_acceptance::{
    AcceptanceFactKind, AcceptanceIngressResult, AcceptanceLaunch, AcceptanceProvider,
    AcceptanceScope, AcceptanceUnavailableReason, ProviderLifecycleAcceptanceHost,
};

const RELEASH_CLI_PATH: &str = env!("CARGO_BIN_EXE_releash");
const SUPPORTED_CLAUDE_VERSION: &str = "2.1.220 (Claude Code)";
const SUPPORTED_CODEX_VERSION: &str = "codex-cli 0.145.0";
static CHARACTERIZATION_GATE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn output(command: &mut Command, label: &str) -> Output {
    command
        .output()
        .unwrap_or_else(|error| panic!("{label} executable is required: {error}"))
}

fn output_with_timeout(command: &mut Command, label: &str, timeout: Duration) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("{label} executable is required: {error}"));
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait().expect("poll Provider CLI").is_some() {
            return child
                .wait_with_output()
                .expect("collect Provider CLI output");
        }
        if Instant::now() >= deadline {
            child.kill().expect("kill timed out Provider CLI");
            let result = child
                .wait_with_output()
                .expect("collect timed out Provider CLI");
            panic!(
                "{label} timed out: stdout={}, stderr={}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr),
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn assert_success(result: &Output, label: &str) {
    assert!(
        result.status.success(),
        "{label} failed: status={:?}, stdout={}, stderr={}",
        result.status.code(),
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
    );
}

fn installed_version(executable: &str) -> String {
    let result = output(Command::new(executable).arg("--version"), executable);
    assert_success(&result, &format!("{executable} --version"));
    String::from_utf8(result.stdout)
        .expect("Provider CLI version must be UTF-8")
        .trim()
        .to_string()
}

fn write_launch_files(root: &Path, launch: &AcceptanceLaunch) {
    for file in &launch.files {
        let path = root.join(&file.relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create generated Provider config directory");
        }
        std::fs::write(path, &file.contents).expect("write generated Provider config");
    }
}

fn install_releash_alias(data_dir: &Path, alias: &str) -> PathBuf {
    assert!(matches!(alias, "releash" | "releash-dev"));
    let bin = data_dir.join("bin");
    std::fs::create_dir_all(&bin).expect("create characterization alias directory");
    let wrapper = bin.join(alias);
    let script = format!("#!/bin/sh\nexec '{}' \"$@\"\n", RELEASH_CLI_PATH);
    std::fs::write(&wrapper, script).expect("write characterization alias wrapper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(&wrapper).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&wrapper, permissions).unwrap();
    }
    bin
}

fn generated_hook_alias(launch: &AcceptanceLaunch) -> String {
    match launch.provider {
        AcceptanceProvider::Claude => {
            let hooks = launch
                .files
                .iter()
                .find(|file| file.relative_path == Path::new("hooks/hooks.json"))
                .expect("Claude hooks config");
            let value: serde_json::Value = serde_json::from_slice(&hooks.contents).unwrap();
            value["hooks"]["SessionStart"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .split_ascii_whitespace()
                .next()
                .unwrap()
                .to_string()
        }
        AcceptanceProvider::Codex => launch
            .arguments
            .iter()
            .find(|argument| argument.starts_with("hooks.SessionStart="))
            .unwrap()
            .split("command=\"")
            .nth(1)
            .unwrap()
            .split_ascii_whitespace()
            .next()
            .unwrap()
            .to_string(),
    }
}

fn apply_launch_environment(command: &mut Command, launch: &AcceptanceLaunch, data_dir: &Path) {
    command.envs(launch.environment.iter().map(|(key, value)| (key, value)));
    let alias = generated_hook_alias(launch);
    let bin = install_releash_alias(data_dir, &alias);
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(&path));
    command.env(
        "PATH",
        std::env::join_paths(paths).expect("compose characterization PATH"),
    );
    command.env("RELEASH_DATA_DIR", data_dir);
}

fn install_user_hook(config_dir: &Path) -> PathBuf {
    let marker = config_dir.join("user-hook-events.jsonl");
    let hook = config_dir.join("user-hook");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\n/bin/cat >> '{}'\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(&hook).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&hook, permissions).unwrap();
    }
    let settings = serde_json::json!({
        "hooks": {
            "SessionStart": [{"hooks": [{"type": "command", "command": hook}]}],
            "Stop": [{"hooks": [{"type": "command", "command": hook}]}]
        }
    });
    std::fs::write(config_dir.join("settings.json"), settings.to_string()).unwrap();
    marker
}

fn user_configuration_paths() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .expect("HOME is required for user configuration invariance");
    vec![
        home.join(".claude/settings.json"),
        home.join(".codex/config.toml"),
        home.join(".codex/hooks.json"),
        home.join(".codex/auth.json"),
    ]
}

fn configuration_snapshot(paths: &[PathBuf]) -> Vec<Option<Vec<u8>>> {
    paths
        .iter()
        .map(|path| match std::fs::read(path) {
            Ok(contents) => Some(contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("failed to snapshot {}: {error}", path.display()),
        })
        .collect()
}

fn install_codex_auth(codex_home: &Path) {
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME is required"));
    let source = home.join(".codex/auth.json");
    assert!(
        source.is_file(),
        "installed Codex characterization requires {}",
        source.display()
    );
    let target = codex_home.join("auth.json");
    std::fs::copy(source, &target).expect("copy Codex auth into isolated CODEX_HOME");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(&target).unwrap().permissions();
        permissions.set_mode(0o600);
        std::fs::set_permissions(target, permissions).unwrap();
    }
}

fn install_codex_user_hooks(codex_home: &Path) -> PathBuf {
    let marker = install_user_hook(codex_home);
    std::fs::rename(
        codex_home.join("settings.json"),
        codex_home.join("hooks.json"),
    )
    .unwrap();
    marker
}

struct CodexPty {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    output: Arc<Mutex<Vec<u8>>>,
}

impl CodexPty {
    fn send(&mut self, input: &[u8]) {
        self.writer.write_all(input).expect("write Codex PTY input");
        self.writer.flush().expect("flush Codex PTY input");
    }

    fn wait_for(&self, marker: &str, timeout: Duration) {
        self.wait_for_any(&[marker], timeout);
    }

    fn wait_for_any(&self, markers: &[&str], timeout: Duration) -> usize {
        let markers = markers
            .iter()
            .map(|marker| marker.to_ascii_lowercase())
            .collect::<Vec<_>>();
        let deadline = Instant::now() + timeout;
        loop {
            let normalized = normalized_terminal_output(&self.output.lock().unwrap());
            if let Some(index) = markers
                .iter()
                .position(|marker| normalized.contains(marker))
            {
                return index;
            }
            assert!(
                Instant::now() < deadline,
                "Codex TUI did not show any of {markers:?}: {}",
                String::from_utf8_lossy(&self.output.lock().unwrap())
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn diagnostic(&self) -> String {
        normalized_terminal_output(&self.output.lock().unwrap())
    }

    fn submit_prompt(&mut self, prompt: &str) {
        self.wait_for(
            "modelgpt56solmodeltochangedirectory",
            Duration::from_secs(30),
        );
        self.send(prompt.as_bytes());
        thread::sleep(Duration::from_millis(50));
        self.send(b"\r");
    }
}

impl Drop for CodexPty {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn normalized_terminal_output(bytes: &[u8]) -> String {
    #[derive(Clone, Copy)]
    enum EscapeState {
        Text,
        Escape,
        Csi,
        Osc,
        OscEscape,
    }

    let mut state = EscapeState::Text;
    let mut normalized = String::new();
    for &byte in bytes {
        state = match state {
            EscapeState::Text if byte == 0x1b => EscapeState::Escape,
            EscapeState::Text => {
                if byte.is_ascii_alphanumeric() {
                    normalized.push(char::from(byte).to_ascii_lowercase());
                }
                EscapeState::Text
            }
            EscapeState::Escape if byte == b'[' => EscapeState::Csi,
            EscapeState::Escape if byte == b']' => EscapeState::Osc,
            EscapeState::Escape => EscapeState::Text,
            EscapeState::Csi if (0x40..=0x7e).contains(&byte) => EscapeState::Text,
            EscapeState::Csi => EscapeState::Csi,
            EscapeState::Osc if byte == 0x07 => EscapeState::Text,
            EscapeState::Osc if byte == 0x1b => EscapeState::OscEscape,
            EscapeState::Osc => EscapeState::Osc,
            EscapeState::OscEscape if byte == b'\\' => EscapeState::Text,
            EscapeState::OscEscape => EscapeState::Osc,
        };
    }
    normalized
}

fn spawn_codex_tui(
    launch: &AcceptanceLaunch,
    data_dir: &Path,
    codex_home: &Path,
    workspace: &Path,
) -> CodexPty {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open Codex characterization PTY");
    let mut command = CommandBuilder::new("codex");
    for argument in &launch.arguments {
        command.arg(argument);
    }
    command.arg("--no-alt-screen");
    command.arg("--disable");
    command.arg("apps");
    command.cwd(workspace);
    command.env("CODEX_HOME", codex_home);
    command.env("TERM", "xterm-256color");
    for (key, value) in &launch.environment {
        command.env(key, value);
    }
    if !launch.arguments.is_empty() {
        let alias = generated_hook_alias(launch);
        let bin = install_releash_alias(data_dir, &alias);
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![bin];
        paths.extend(std::env::split_paths(&path));
        command.env("PATH", std::env::join_paths(paths).unwrap());
    }
    command.env("RELEASH_DATA_DIR", data_dir);

    let child = pair
        .slave
        .spawn_command(command)
        .expect("spawn installed Codex TUI");
    drop(pair.slave);
    let writer = pair.master.take_writer().expect("take Codex PTY writer");
    let mut reader = pair
        .master
        .try_clone_reader()
        .expect("clone Codex PTY reader");
    let output = Arc::new(Mutex::new(Vec::new()));
    let thread_output = output.clone();
    thread::spawn(move || {
        let mut buffer = [0_u8; 8_192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => thread_output
                    .lock()
                    .unwrap()
                    .extend_from_slice(&buffer[..read]),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) if error.raw_os_error() == Some(5) => break,
                Err(_) => break,
            }
        }
    });
    CodexPty {
        child,
        writer,
        output,
    }
}

async fn wait_for_fact(
    host: &ProviderLifecycleAcceptanceHost,
    agent_session_id: &str,
    predicate: impl Fn(&AcceptanceFactKind) -> bool,
    timeout: Duration,
    diagnostic: impl Fn() -> String,
) {
    let deadline = Instant::now() + timeout;
    loop {
        let facts = host.facts(agent_session_id).await.unwrap();
        if facts.iter().any(|fact| predicate(&fact.kind)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for durable Provider lifecycle fact: {facts:?}; provider output: {}",
            diagnostic(),
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn wait_for_user_hook_event(
    marker: &Path,
    event: &str,
    timeout: Duration,
    diagnostic: impl Fn() -> String,
) {
    let expected = format!("\"hook_event_name\":\"{event}\"");
    let deadline = Instant::now() + timeout;
    loop {
        if std::fs::read_to_string(marker).is_ok_and(|contents| contents.contains(&expected)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "user Hook did not observe {event}: {}",
            diagnostic(),
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "installed Claude Code / Codex CLI characterization gate"]
async fn test_providerライフサイクル実cli検証_supported_installed_cliがgenerated_configを受理する()
{
    let _gate = CHARACTERIZATION_GATE_LOCK.lock().await;
    assert_eq!(installed_version("claude"), SUPPORTED_CLAUDE_VERSION);
    assert_eq!(installed_version("codex"), SUPPORTED_CODEX_VERSION);

    let user_paths = user_configuration_paths();
    let before = configuration_snapshot(&user_paths);
    let data_dir = tempfile::TempDir::new().unwrap();
    let host = ProviderLifecycleAcceptanceHost::start(data_dir.path()).unwrap();

    let claude_plugin = tempfile::TempDir::new().unwrap();
    let claude_launch = host
        .prepare_launch(
            AcceptanceProvider::Claude,
            AcceptanceScope::new(
                "agent-characterization-claude-config",
                "workflow-characterization-claude-config",
                "node-characterization-claude-config",
                1,
            ),
            Some(claude_plugin.path()),
        )
        .await
        .unwrap();
    write_launch_files(claude_plugin.path(), &claude_launch);
    let claude_config = tempfile::TempDir::new().unwrap();
    let claude_result = output(
        Command::new("claude")
            .args(claude_launch.arguments.iter())
            .arg("doctor")
            .env("CLAUDE_CONFIG_DIR", claude_config.path()),
        "Claude generated plugin validation",
    );
    assert_success(&claude_result, "Claude generated plugin validation");

    let codex_launch = host
        .prepare_launch(
            AcceptanceProvider::Codex,
            AcceptanceScope::new(
                "agent-characterization-codex-config",
                "workflow-characterization-codex-config",
                "node-characterization-codex-config",
                1,
            ),
            None,
        )
        .await
        .unwrap();
    assert!(!codex_launch
        .arguments
        .iter()
        .any(|argument| argument.contains("dangerously-bypass-hook-trust")));
    let codex_home = tempfile::TempDir::new().unwrap();
    let codex_result = output(
        Command::new("codex")
            .arg("doctor")
            .args(codex_launch.arguments.iter())
            .args(["--summary", "--ascii"])
            .env("CODEX_HOME", codex_home.path()),
        "Codex generated per-process config validation",
    );
    let codex_diagnostic = String::from_utf8_lossy(&codex_result.stdout);
    assert!(
        codex_diagnostic.contains("[ok] config") && codex_diagnostic.contains("loaded"),
        "Codex rejected generated per-process config: stdout={}, stderr={}",
        codex_diagnostic,
        String::from_utf8_lossy(&codex_result.stderr),
    );

    assert_eq!(configuration_snapshot(&user_paths), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "installed Claude Code / Codex CLI characterization gate"]
async fn test_providerライフサイクル実cli検証_claude実cliでuser_hookとsession_startが共存する() {
    let _gate = CHARACTERIZATION_GATE_LOCK.lock().await;
    assert_eq!(installed_version("claude"), SUPPORTED_CLAUDE_VERSION);
    let user_paths = user_configuration_paths();
    let before = configuration_snapshot(&user_paths);
    let data_dir = tempfile::TempDir::new().unwrap();
    let plugin = tempfile::TempDir::new().unwrap();
    let config = tempfile::TempDir::new().unwrap();
    let host = ProviderLifecycleAcceptanceHost::start(data_dir.path()).unwrap();
    let scope = AcceptanceScope::new(
        "agent-characterization-claude-session-start",
        "workflow-characterization-claude-session-start",
        "node-characterization-claude-session-start",
        1,
    );
    let launch = host
        .prepare_launch(
            AcceptanceProvider::Claude,
            scope.clone(),
            Some(plugin.path()),
        )
        .await
        .unwrap();
    write_launch_files(plugin.path(), &launch);
    let marker = install_user_hook(config.path());
    let settings_before = std::fs::read(config.path().join("settings.json")).unwrap();

    let mut command = Command::new("claude");
    command
        .args(launch.arguments.iter())
        .arg("--init-only")
        .env("CLAUDE_CONFIG_DIR", config.path())
        .env("TERM", "dumb");
    apply_launch_environment(&mut command, &launch, data_dir.path());
    let result = output_with_timeout(
        &mut command,
        "Claude SessionStart characterization",
        Duration::from_secs(30),
    );
    assert_success(&result, "Claude SessionStart characterization");

    let user_events = std::fs::read_to_string(marker).expect("user SessionStart Hook must run");
    assert!(user_events.contains("\"hook_event_name\":\"SessionStart\""));
    let facts = host.facts(&scope.agent_session_id).await.unwrap();
    assert!(facts
        .iter()
        .any(|fact| { matches!(fact.kind, AcceptanceFactKind::SessionAssociated { .. }) }));
    assert_eq!(
        std::fs::read(config.path().join("settings.json")).unwrap(),
        settings_before
    );
    assert_eq!(configuration_snapshot(&user_paths), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "installed Claude Code / Codex CLI characterization gate"]
async fn test_providerライフサイクル実cli検証_claudeのsession_startとstopがdurableになる() {
    let _gate = CHARACTERIZATION_GATE_LOCK.lock().await;
    assert_eq!(installed_version("claude"), SUPPORTED_CLAUDE_VERSION);
    let user_paths = user_configuration_paths();
    let before = configuration_snapshot(&user_paths);
    let data_dir = tempfile::TempDir::new().unwrap();
    let plugin = tempfile::TempDir::new().unwrap();
    let config = tempfile::TempDir::new().unwrap();
    let host = ProviderLifecycleAcceptanceHost::start(data_dir.path()).unwrap();
    let scope = AcceptanceScope::new(
        "agent-characterization-claude-stop",
        "workflow-characterization-claude-stop",
        "node-characterization-claude-stop",
        1,
    );
    let launch = host
        .prepare_launch(
            AcceptanceProvider::Claude,
            scope.clone(),
            Some(plugin.path()),
        )
        .await
        .unwrap();
    write_launch_files(plugin.path(), &launch);
    let marker = install_user_hook(config.path());
    let settings_before = std::fs::read(config.path().join("settings.json")).unwrap();

    let mut command = Command::new("claude");
    command
        .args(launch.arguments.iter())
        .args(["--print", "--no-session-persistence", "--settings"])
        .arg(config.path().join("settings.json"))
        .args(["--tools", "", "--model", "haiku", "Reply with exactly ok."])
        .env("TERM", "dumb");
    apply_launch_environment(&mut command, &launch, data_dir.path());
    let result = output_with_timeout(
        &mut command,
        "Claude Stop characterization",
        Duration::from_secs(120),
    );
    assert_success(&result, "Claude Stop characterization");

    let user_events = std::fs::read_to_string(marker).expect("user lifecycle Hooks must run");
    assert!(user_events.contains("\"hook_event_name\":\"SessionStart\""));
    assert!(user_events.contains("\"hook_event_name\":\"Stop\""));
    let facts = host.facts(&scope.agent_session_id).await.unwrap();
    assert!(facts
        .iter()
        .any(|fact| { matches!(fact.kind, AcceptanceFactKind::SessionAssociated { .. }) }));
    assert!(facts
        .iter()
        .any(|fact| matches!(fact.kind, AcceptanceFactKind::StopObserved { .. })));
    assert_eq!(
        std::fs::read(config.path().join("settings.json")).unwrap(),
        settings_before
    );
    assert_eq!(configuration_snapshot(&user_paths), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installed Claude Code / Codex CLI characterization gate"]
async fn test_providerライフサイクル実cli検証_codex実tuiのtrust後session_startとstopがdurableになる(
) {
    let _gate = CHARACTERIZATION_GATE_LOCK.lock().await;
    assert_eq!(installed_version("codex"), SUPPORTED_CODEX_VERSION);
    let user_paths = user_configuration_paths();
    let before = configuration_snapshot(&user_paths);
    let data_dir = tempfile::TempDir::new().unwrap();
    let codex_home = tempfile::TempDir::new().unwrap();
    let workspace = tempfile::TempDir::new().unwrap();
    install_codex_auth(codex_home.path());
    let marker = install_codex_user_hooks(codex_home.path());
    let host = ProviderLifecycleAcceptanceHost::start(data_dir.path()).unwrap();
    let scope = AcceptanceScope::new(
        "agent-characterization-codex-stop",
        "workflow-characterization-codex-stop",
        "node-characterization-codex-stop",
        1,
    );
    let launch = host
        .prepare_launch(AcceptanceProvider::Codex, scope.clone(), None)
        .await
        .unwrap();
    assert!(!launch
        .arguments
        .iter()
        .any(|argument| argument.contains("dangerously-bypass-hook-trust")));

    let mut codex = spawn_codex_tui(
        &launch,
        data_dir.path(),
        codex_home.path(),
        workspace.path(),
    );
    codex.wait_for(
        "doyoutrustthecontentsofthisdirectory",
        Duration::from_secs(30),
    );
    codex.send(b"\r");
    codex.wait_for("hooksneedreview", Duration::from_secs(30));
    codex.send(b"\x1b[B\r");
    codex.wait_for("pressentertoconfirm", Duration::from_secs(30));
    codex.send(b"\r");
    codex.wait_for(
        "modelgpt56solmodeltochangedirectory",
        Duration::from_secs(30),
    );
    let facts_before_trusted_restart = host.facts(&scope.agent_session_id).await.unwrap();
    assert!(facts_before_trusted_restart
        .iter()
        .all(|fact| !matches!(fact.kind, AcceptanceFactKind::SessionAssociated { .. })));
    drop(codex);

    let mut codex = spawn_codex_tui(
        &launch,
        data_dir.path(),
        codex_home.path(),
        workspace.path(),
    );
    if codex.wait_for_any(
        &["skipuntilnextversion", "openaicodexv01450"],
        Duration::from_secs(30),
    ) == 0
    {
        codex.send(b"\x1b[B\r");
    }
    codex.submit_prompt("Reply with exactly ok.");

    wait_for_fact(
        &host,
        &scope.agent_session_id,
        |kind| matches!(kind, AcceptanceFactKind::SessionAssociated { .. }),
        Duration::from_secs(30),
        || codex.diagnostic(),
    )
    .await;
    wait_for_fact(
        &host,
        &scope.agent_session_id,
        |kind| matches!(kind, AcceptanceFactKind::StopObserved { .. }),
        Duration::from_secs(120),
        || codex.diagnostic(),
    )
    .await;
    assert!(codex.child.try_wait().unwrap().is_none());

    let user_events = std::fs::read_to_string(marker).expect("Codex user Hooks must run");
    assert!(user_events.contains("\"hook_event_name\":\"SessionStart\""));
    assert!(user_events.contains("\"hook_event_name\":\"Stop\""));
    assert_eq!(configuration_snapshot(&user_paths), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installed Claude Code / Codex CLI characterization gate"]
async fn test_providerライフサイクル実cli検証_codex_hook_trust未確認をdurableに診断する() {
    let _gate = CHARACTERIZATION_GATE_LOCK.lock().await;
    assert_eq!(installed_version("codex"), SUPPORTED_CODEX_VERSION);
    let user_paths = user_configuration_paths();
    let before = configuration_snapshot(&user_paths);
    let data_dir = tempfile::TempDir::new().unwrap();
    let codex_home = tempfile::TempDir::new().unwrap();
    let workspace = tempfile::TempDir::new().unwrap();
    install_codex_auth(codex_home.path());
    let marker = install_codex_user_hooks(codex_home.path());
    let host = ProviderLifecycleAcceptanceHost::start(data_dir.path()).unwrap();
    let scope = AcceptanceScope::new(
        "agent-characterization-codex-untrusted",
        "workflow-characterization-codex-untrusted",
        "node-characterization-codex-untrusted",
        1,
    );
    let launch = host
        .prepare_launch(AcceptanceProvider::Codex, scope.clone(), None)
        .await
        .unwrap();
    assert!(launch.requires_hook_trust);
    assert!(!launch
        .arguments
        .iter()
        .any(|argument| argument.contains("dangerously-bypass-hook-trust")));

    let mut codex = spawn_codex_tui(
        &launch,
        data_dir.path(),
        codex_home.path(),
        workspace.path(),
    );
    codex.wait_for(
        "doyoutrustthecontentsofthisdirectory",
        Duration::from_secs(30),
    );
    codex.send(b"\r");
    codex.wait_for("hooksneedreview", Duration::from_secs(30));
    codex.send(b"\x1b[B\x1b[B\r");
    codex.wait_for("openaicodexv01450", Duration::from_secs(30));

    assert_eq!(
        host.report_unavailable(
            &launch,
            AcceptanceUnavailableReason::CodexHookDeliveryUnconfirmed,
        )
        .await
        .unwrap(),
        AcceptanceIngressResult::Applied,
    );

    let facts = host.facts(&scope.agent_session_id).await.unwrap();
    assert!(facts
        .iter()
        .all(|fact| !matches!(fact.kind, AcceptanceFactKind::SessionAssociated { .. })));
    assert!(facts.iter().any(|fact| {
        matches!(
            &fact.kind,
            AcceptanceFactKind::LifecycleUnavailable { reason, .. }
                if reason == "codex_hook_delivery_unconfirmed"
        )
    }));
    assert!(codex.child.try_wait().unwrap().is_none());
    assert!(!marker.exists());
    assert_eq!(configuration_snapshot(&user_paths), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installed Claude Code / Codex CLI characterization gate"]
async fn test_providerライフサイクル実cli検証_releash_hookをreleash起動processだけに適用する() {
    let _gate = CHARACTERIZATION_GATE_LOCK.lock().await;
    assert_eq!(installed_version("claude"), SUPPORTED_CLAUDE_VERSION);
    assert_eq!(installed_version("codex"), SUPPORTED_CODEX_VERSION);
    let user_paths = user_configuration_paths();
    let before = configuration_snapshot(&user_paths);
    let data_dir = tempfile::TempDir::new().unwrap();
    let host = ProviderLifecycleAcceptanceHost::start(data_dir.path()).unwrap();

    let claude_plugin = tempfile::TempDir::new().unwrap();
    let claude_config = tempfile::TempDir::new().unwrap();
    let claude_scope = AcceptanceScope::new(
        "agent-characterization-claude-unmanaged",
        "workflow-characterization-claude-unmanaged",
        "node-characterization-claude-unmanaged",
        1,
    );
    let _claude_launch = host
        .prepare_launch(
            AcceptanceProvider::Claude,
            claude_scope.clone(),
            Some(claude_plugin.path()),
        )
        .await
        .unwrap();
    let claude_user_marker = install_user_hook(claude_config.path());
    let claude_result = output_with_timeout(
        Command::new("claude")
            .arg("--init-only")
            .env("CLAUDE_CONFIG_DIR", claude_config.path())
            .env("RELEASH_DATA_DIR", data_dir.path())
            .env("TERM", "dumb"),
        "Claude process without Releash launch configuration",
        Duration::from_secs(30),
    );
    assert_success(
        &claude_result,
        "Claude process without Releash launch configuration",
    );
    let claude_user_events = std::fs::read_to_string(claude_user_marker).unwrap();
    assert!(claude_user_events.contains("\"hook_event_name\":\"SessionStart\""));
    let claude_facts = host.facts(&claude_scope.agent_session_id).await.unwrap();
    assert!(claude_facts
        .iter()
        .all(|fact| !matches!(fact.kind, AcceptanceFactKind::SessionAssociated { .. })));

    let codex_home = tempfile::TempDir::new().unwrap();
    let codex_workspace = tempfile::TempDir::new().unwrap();
    install_codex_auth(codex_home.path());
    let codex_user_marker = install_codex_user_hooks(codex_home.path());
    let codex_scope = AcceptanceScope::new(
        "agent-characterization-codex-unmanaged",
        "workflow-characterization-codex-unmanaged",
        "node-characterization-codex-unmanaged",
        1,
    );
    let mut without_releash = host
        .prepare_launch(AcceptanceProvider::Codex, codex_scope.clone(), None)
        .await
        .unwrap();
    without_releash.arguments.clear();
    without_releash.environment.clear();
    let mut codex = spawn_codex_tui(
        &without_releash,
        data_dir.path(),
        codex_home.path(),
        codex_workspace.path(),
    );
    codex.wait_for(
        "doyoutrustthecontentsofthisdirectory",
        Duration::from_secs(30),
    );
    codex.send(b"\r");
    codex.wait_for("2hooksareneworchanged", Duration::from_secs(30));
    codex.send(b"\x1b[B\r");
    codex.wait_for("pressentertoconfirm", Duration::from_secs(30));
    codex.send(b"\r");
    codex.wait_for(
        "modelgpt56solmodeltochangedirectory",
        Duration::from_secs(30),
    );
    drop(codex);

    let mut codex = spawn_codex_tui(
        &without_releash,
        data_dir.path(),
        codex_home.path(),
        codex_workspace.path(),
    );
    if codex.wait_for_any(
        &["skipuntilnextversion", "openaicodexv01450"],
        Duration::from_secs(30),
    ) == 0
    {
        codex.send(b"\x1b[B\r");
    }
    codex.submit_prompt("Reply with exactly ok.");
    wait_for_user_hook_event(
        &codex_user_marker,
        "SessionStart",
        Duration::from_secs(30),
        || codex.diagnostic(),
    );
    let codex_facts = host.facts(&codex_scope.agent_session_id).await.unwrap();
    assert!(codex_facts
        .iter()
        .all(|fact| !matches!(fact.kind, AcceptanceFactKind::SessionAssociated { .. })));
    assert!(codex.child.try_wait().unwrap().is_none());
    assert_eq!(configuration_snapshot(&user_paths), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installed Claude Code / Codex CLI characterization gate"]
async fn test_providerライフサイクル実cli検証_local_api不在でもproviderを強制終了しない() {
    let _gate = CHARACTERIZATION_GATE_LOCK.lock().await;
    assert_eq!(installed_version("claude"), SUPPORTED_CLAUDE_VERSION);
    assert_eq!(installed_version("codex"), SUPPORTED_CODEX_VERSION);
    let user_paths = user_configuration_paths();
    let before = configuration_snapshot(&user_paths);
    let data_dir = tempfile::TempDir::new().unwrap();
    let unavailable_data_dir = tempfile::TempDir::new().unwrap();
    let host = ProviderLifecycleAcceptanceHost::start(data_dir.path()).unwrap();

    let claude_plugin = tempfile::TempDir::new().unwrap();
    let claude_config = tempfile::TempDir::new().unwrap();
    let claude_scope = AcceptanceScope::new(
        "agent-characterization-claude-api-unavailable",
        "workflow-characterization-claude-api-unavailable",
        "node-characterization-claude-api-unavailable",
        1,
    );
    let claude_launch = host
        .prepare_launch(
            AcceptanceProvider::Claude,
            claude_scope.clone(),
            Some(claude_plugin.path()),
        )
        .await
        .unwrap();
    write_launch_files(claude_plugin.path(), &claude_launch);
    let claude_user_marker = install_user_hook(claude_config.path());
    let mut claude_command = Command::new("claude");
    claude_command
        .args(claude_launch.arguments.iter())
        .arg("--init-only")
        .env("CLAUDE_CONFIG_DIR", claude_config.path())
        .env("TERM", "dumb");
    apply_launch_environment(
        &mut claude_command,
        &claude_launch,
        unavailable_data_dir.path(),
    );
    let claude_result = output_with_timeout(
        &mut claude_command,
        "Claude Local API failure characterization",
        Duration::from_secs(30),
    );
    assert_success(&claude_result, "Claude Local API failure characterization");
    let claude_user_events = std::fs::read_to_string(claude_user_marker).unwrap();
    assert!(claude_user_events.contains("\"hook_event_name\":\"SessionStart\""));
    let claude_facts = host.facts(&claude_scope.agent_session_id).await.unwrap();
    assert!(claude_facts
        .iter()
        .all(|fact| !matches!(fact.kind, AcceptanceFactKind::SessionAssociated { .. })));

    let codex_home = tempfile::TempDir::new().unwrap();
    let codex_workspace = tempfile::TempDir::new().unwrap();
    install_codex_auth(codex_home.path());
    let codex_user_marker = install_codex_user_hooks(codex_home.path());
    let codex_scope = AcceptanceScope::new(
        "agent-characterization-codex-api-unavailable",
        "workflow-characterization-codex-api-unavailable",
        "node-characterization-codex-api-unavailable",
        1,
    );
    let codex_launch = host
        .prepare_launch(AcceptanceProvider::Codex, codex_scope.clone(), None)
        .await
        .unwrap();
    let mut codex = spawn_codex_tui(
        &codex_launch,
        unavailable_data_dir.path(),
        codex_home.path(),
        codex_workspace.path(),
    );
    codex.wait_for(
        "doyoutrustthecontentsofthisdirectory",
        Duration::from_secs(30),
    );
    codex.send(b"\r");
    codex.wait_for("hooksneedreview", Duration::from_secs(30));
    codex.send(b"\x1b[B\r");
    codex.wait_for("pressentertoconfirm", Duration::from_secs(30));
    codex.send(b"\r");
    codex.wait_for(
        "modelgpt56solmodeltochangedirectory",
        Duration::from_secs(30),
    );
    drop(codex);

    let mut codex = spawn_codex_tui(
        &codex_launch,
        unavailable_data_dir.path(),
        codex_home.path(),
        codex_workspace.path(),
    );
    if codex.wait_for_any(
        &["skipuntilnextversion", "openaicodexv01450"],
        Duration::from_secs(30),
    ) == 0
    {
        codex.send(b"\x1b[B\r");
    }
    codex.submit_prompt("Reply with exactly ok.");
    wait_for_user_hook_event(
        &codex_user_marker,
        "SessionStart",
        Duration::from_secs(30),
        || codex.diagnostic(),
    );
    assert!(codex.child.try_wait().unwrap().is_none());
    let codex_facts = host.facts(&codex_scope.agent_session_id).await.unwrap();
    assert!(codex_facts.iter().all(|fact| {
        !matches!(
            fact.kind,
            AcceptanceFactKind::SessionAssociated { .. } | AcceptanceFactKind::StopObserved { .. }
        )
    }));
    assert_eq!(configuration_snapshot(&user_paths), before);
}
