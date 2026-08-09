#[path = "support/agent_tui_fixture.rs"]
mod agent_tui_fixture;

use std::time::Duration;

use agent_tui_fixture::{fixture_process_launch, fixture_process_shell_command, FixturePlan};
use releash_lib::terminal_surface::{
    TerminalProcessLaunchV1, TerminalSurfaceEventFault, TerminalSurfaceOwnerV1,
    TerminalSurfaceRuntime, TerminalSurfaceStreamItemV1, TerminalSurfaceV1,
    TerminalSurfaceWireAttachment,
};

const PRODUCTION_SCROLLBACK_ROWS: usize = 1_000;
const RECONSTRUCTION_SCROLLBACK_ROWS: usize = 10_000;

fn workspace_owner(path: &str) -> TerminalSurfaceOwnerV1 {
    TerminalSurfaceOwnerV1::Workspace {
        workspace_path: path.to_string(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_030_provider_cliがterminal_surfaceのroot_processとして終了する() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let cwd = tempfile::TempDir::new().unwrap();
    let path = cwd.path().to_string_lossy().into_owned();
    let owner = session_owner(&path, "agent-session-root-process");
    let (_app, runtime) = build_runtime(data_dir.path());
    let fixture = FixturePlan {
        input_lines: 1,
        alternate_screen: true,
        ..FixturePlan::new("atui-030-root-process", vec![])
    };
    let launch = fixture_process_launch(&fixture);

    runtime
        .get_or_spawn_with_process(
            24,
            80,
            Some(path),
            owner.clone(),
            Some("AgentSession root process".to_string()),
            TerminalProcessLaunchV1 {
                executable: launch.executable,
                arguments: launch.arguments,
                environment: launch.environment,
            },
        )
        .expect("spawn provider fixture as PTY root process");
    let mut attachment = runtime
        .attach("atui-030-root-process".to_string(), owner.clone())
        .expect("attach root process surface");
    receive_until(&mut attachment, "atui-030-root-process").await;
    runtime
        .write(owner.clone(), "operator-input\r")
        .expect("write provider input");
    receive_until(&mut attachment, "received-0:operator-input").await;
    receive_exit(&mut attachment).await;

    assert!(runtime.get(owner.clone()).unwrap().is_exited);
    assert!(runtime.write(owner, "echo shell-remained\r").is_err());
}

fn session_owner(path: &str, session_id: &str) -> TerminalSurfaceOwnerV1 {
    TerminalSurfaceOwnerV1::Session {
        workspace_path: path.to_string(),
        session_id: session_id.to_string(),
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

async fn wait_file_content(path: &std::path::Path) -> String {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(content) = std::fs::read_to_string(path) {
                if !content.is_empty() {
                    return content;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for file content: {}", path.display()))
}

async fn wait_file_content_equals(path: &std::path::Path, expected: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if std::fs::read_to_string(path).ok().as_deref() == Some(expected) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "timed out waiting for exact file content {expected:?}: {}",
            path.display()
        )
    });
}

#[cfg(unix)]
async fn wait_process_exit(pid: &str) {
    let pid = pid.trim().parse::<libc::pid_t>().expect("parse shell pid");
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let result = unsafe { libc::kill(pid, 0) };
            if result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("old Terminal Surface process remained alive: {pid}"));
}

fn build_runtime(
    data_dir: &std::path::Path,
) -> (tauri::App<tauri::test::MockRuntime>, TerminalSurfaceRuntime) {
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("build Tauri product-path app");
    let runtime =
        TerminalSurfaceRuntime::new_with_data_dir(app.handle().clone(), data_dir.to_path_buf());
    (app, runtime)
}

async fn receive_until(
    attachment: &mut TerminalSurfaceWireAttachment,
    needle: &str,
) -> Vec<TerminalSurfaceStreamItemV1> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut items = Vec::new();
        let mut output = String::new();
        while !output.contains(needle) {
            let item = attachment.next().await.unwrap_or_else(|| {
                panic!("Terminal Surface stream closed before: {needle}; output: {output:?}")
            });
            match &item {
                TerminalSurfaceStreamItemV1::Snapshot { surface } => {
                    output.push_str(&surface.terminal_surface.replay);
                }
                TerminalSurfaceStreamItemV1::Output { data, .. } => output.push_str(data),
                _ => {}
            }
            items.push(item);
        }
        items
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for Terminal Surface output: {needle}"))
}

async fn receive_exit(
    attachment: &mut TerminalSurfaceWireAttachment,
) -> Vec<TerminalSurfaceStreamItemV1> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut items = Vec::new();
        loop {
            let item = attachment
                .next()
                .await
                .expect("Terminal Surface exit stream");
            let exited = matches!(item, TerminalSurfaceStreamItemV1::Exit { .. });
            items.push(item);
            if exited {
                return items;
            }
        }
    })
    .await
    .expect("timed out waiting for Terminal Surface exit")
}

async fn wait_surface_contains(
    runtime: &TerminalSurfaceRuntime,
    owner: &TerminalSurfaceOwnerV1,
    needle: &str,
) {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let surface = runtime.get(owner.clone()).expect("read Terminal Surface");
            if surface_text(&surface).contains(needle) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    if result.is_err() {
        let replay = runtime
            .get(owner.clone())
            .map(|surface| surface.terminal_surface.replay)
            .unwrap_or_else(|error| format!("failed to read surface: {error}"));
        panic!("timed out waiting for Terminal Surface checkpoint: {needle}; replay: {replay:?}");
    }
}

async fn wait_surface_cursor(
    runtime: &TerminalSurfaceRuntime,
    owner: &TerminalSurfaceOwnerV1,
    expected: (usize, usize),
) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let surface = runtime.get(owner.clone()).expect("read Terminal Surface");
            let terminal = restore_surface(&surface);
            if terminal.cursor() == expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for Terminal Surface cursor: {expected:?}"));
}

fn restore_surface(surface: &TerminalSurfaceV1) -> avt::Vt {
    let mut terminal = avt::Vt::builder()
        .size(
            usize::from(surface.terminal_surface.cols),
            usize::from(surface.terminal_surface.rows),
        )
        .scrollback_limit(RECONSTRUCTION_SCROLLBACK_ROWS)
        .build();
    terminal.feed_str(&surface.terminal_surface.replay);
    terminal
}

fn surface_text(surface: &TerminalSurfaceV1) -> String {
    restore_surface(surface).text().join("\n")
}

fn snapshot(items: &[TerminalSurfaceStreamItemV1]) -> TerminalSurfaceV1 {
    items
        .iter()
        .find_map(|item| match item {
            TerminalSurfaceStreamItemV1::Snapshot { surface } => Some(surface.clone()),
            _ => None,
        })
        .expect("attachment starts with a snapshot")
}

fn output_events(items: &[TerminalSurfaceStreamItemV1]) -> Vec<(u64, String)> {
    items
        .iter()
        .filter_map(|item| match item {
            TerminalSurfaceStreamItemV1::Output { data, sequence, .. } => {
                Some((*sequence, data.to_string()))
            }
            _ => None,
        })
        .collect()
}

fn reconstruct(surface: &TerminalSurfaceV1, events: &[(u64, String)]) -> Result<String, String> {
    let mut expected = surface.terminal_surface.sequence;
    let mut terminal = avt::Vt::builder()
        .size(
            usize::from(surface.terminal_surface.cols),
            usize::from(surface.terminal_surface.rows),
        )
        .scrollback_limit(RECONSTRUCTION_SCROLLBACK_ROWS)
        .build();
    terminal.feed_str(&surface.terminal_surface.replay);
    for (sequence, data) in events {
        if *sequence != expected + 1 {
            return Err(format!(
                "expected sequence {}, received {sequence}",
                expected + 1
            ));
        }
        expected = *sequence;
        terminal.feed_str(data);
    }
    Ok(terminal.text().join("\n"))
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_010_実ptyのproduction_attachが欠落重複逆転なく再接続する() {
    const FRAME_COUNT: usize = 20;
    const ATTACH_BOUNDARY: usize = FRAME_COUNT / 2;
    const DETACH_BOUNDARY: usize = 14;
    const REATTACH_BOUNDARY: usize = 17;

    let data_dir = tempfile::TempDir::new().unwrap();
    let cwd = tempfile::TempDir::new().unwrap();
    let path = cwd.path().to_string_lossy().into_owned();
    let owner = workspace_owner(&path);
    let (_app, runtime) = build_runtime(data_dir.path());
    let spawned = runtime
        .get_or_spawn(
            24,
            2_000,
            Some(path.clone()),
            owner.clone(),
            Some("ATUI-010 fixture".to_string()),
        )
        .expect("spawn production PTY");
    let fixture = FixturePlan {
        input_lines: FRAME_COUNT,
        alternate_screen: false,
        ..FixturePlan::new("atui-010-live-attach", vec![])
    };
    runtime
        .write(
            owner.clone(),
            &format!("{}\n", fixture_process_shell_command(&fixture)),
        )
        .expect("launch fixture");
    wait_surface_contains(&runtime, &owner, "atui-010-live-attach").await;

    let markers = (0..FRAME_COUNT)
        .map(|index| format!("frame-{index:03}-{}", "x".repeat(200)))
        .collect::<Vec<_>>();
    for (index, marker) in markers.iter().enumerate().take(ATTACH_BOUNDARY) {
        runtime
            .write(owner.clone(), &format!("{marker}\r"))
            .expect("write pre-attach frame");
        wait_surface_contains(&runtime, &owner, &format!("received-{index}:")).await;
    }

    let mut attached = runtime
        .attach("atui-010-production".to_string(), owner.clone())
        .expect("production attach");
    let first = attached.next().await.expect("initial snapshot");
    let attached_surface = snapshot(std::slice::from_ref(&first));
    assert_eq!(attached_surface.session_key, spawned.session_key);

    let mut forwarded = vec![first];
    for (index, marker) in markers
        .iter()
        .enumerate()
        .take(DETACH_BOUNDARY)
        .skip(ATTACH_BOUNDARY)
    {
        runtime
            .write(owner.clone(), &format!("{marker}\r"))
            .expect("write first live frame");
        forwarded.extend(receive_until(&mut attached, &format!("received-{index}:{marker}")).await);
    }
    let events = output_events(&forwarded);
    assert!(
        events.len() >= 2,
        "mutation oracle requires multiple live events"
    );
    reconstruct(&attached_surface, &events).expect("first production stream converges");

    let mut gap = events.clone();
    gap.remove(0);
    assert!(reconstruct(&attached_surface, &gap).is_err());
    let mut duplicate = events.clone();
    duplicate.insert(1, duplicate[0].clone());
    assert!(reconstruct(&attached_surface, &duplicate).is_err());
    let mut reversal = events.clone();
    reversal.swap(0, 1);
    assert!(reconstruct(&attached_surface, &reversal).is_err());

    drop(attached);
    for (index, marker) in markers
        .iter()
        .enumerate()
        .take(REATTACH_BOUNDARY)
        .skip(DETACH_BOUNDARY)
    {
        runtime
            .write(owner.clone(), &format!("{marker}\r"))
            .expect("write detached frame");
        wait_surface_contains(&runtime, &owner, &format!("received-{index}:{marker}")).await;
    }

    let mut reloaded = runtime
        .attach("atui-010-reload".to_string(), owner.clone())
        .expect("reattach after renderer reload");
    let first_reloaded = reloaded.next().await.expect("reload snapshot");
    let reloaded_surface = snapshot(std::slice::from_ref(&first_reloaded));
    assert_eq!(reloaded_surface.session_key, spawned.session_key);
    assert!(surface_text(&reloaded_surface).contains(&format!(
        "received-{}:{}",
        REATTACH_BOUNDARY - 1,
        markers[REATTACH_BOUNDARY - 1]
    )));
    let mut reloaded_items = vec![first_reloaded];
    for (index, marker) in markers.iter().enumerate().skip(REATTACH_BOUNDARY) {
        runtime
            .write(owner.clone(), &format!("{marker}\r"))
            .expect("write second live frame");
        reloaded_items
            .extend(receive_until(&mut reloaded, &format!("received-{index}:{marker}")).await);
    }
    let text = reconstruct(&reloaded_surface, &output_events(&reloaded_items))
        .expect("reattached production stream converges");
    for (index, marker) in markers.iter().enumerate() {
        assert!(
            text.contains(&format!("received-{index}:{marker}")),
            "reconstructed surface is missing frame {index}"
        );
    }
    runtime.kill(owner).expect("kill product PTY");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_010_実ptyのproduction_attachが注入された欠落重複逆転を判定する() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let cwd = tempfile::TempDir::new().unwrap();
    let path = cwd.path().to_string_lossy().into_owned();
    let owner = session_owner(&path, "fault-injection");
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("build Tauri product-path app");
    let (runtime, faults) = TerminalSurfaceRuntime::new_with_data_dir_and_event_faults(
        app.handle().clone(),
        data_dir.path().to_path_buf(),
    );
    runtime
        .get_or_spawn(24, 240, Some(path), owner.clone(), None)
        .expect("spawn fault-injection PTY");
    let fixture = FixturePlan {
        input_lines: 8,
        alternate_screen: false,
        ..FixturePlan::new("atui-010-fault-injection", vec![])
    };
    runtime
        .write(
            owner.clone(),
            &format!("{}\n", fixture_process_shell_command(&fixture)),
        )
        .expect("launch fault-injection fixture");
    wait_surface_contains(&runtime, &owner, "atui-010-fault-injection").await;
    let mut attached = runtime
        .attach("atui-010-faults".to_string(), owner.clone())
        .expect("attach production continuity detector");
    assert!(matches!(
        attached.next().await,
        Some(TerminalSurfaceStreamItemV1::Snapshot { .. })
    ));

    faults.arm(TerminalSurfaceEventFault::DuplicateNext);
    runtime
        .write(owner.clone(), "fault-duplicate\r")
        .expect("write duplicated production event");
    receive_until(&mut attached, "received-0:fault-duplicate").await;
    runtime
        .write(owner.clone(), "after-duplicate\r")
        .expect("write after duplicated production event");
    let after_duplicate = receive_until(&mut attached, "received-1:after-duplicate").await;
    assert!(!after_duplicate.iter().any(|item| matches!(
        item,
        TerminalSurfaceStreamItemV1::Output { data, .. }
            if data.contains("received-0:fault-duplicate")
    )));

    faults.arm(TerminalSurfaceEventFault::DropNext);
    runtime
        .write(owner.clone(), "fault-gap\r")
        .expect("write dropped production event");
    // 出力は2msバッチで合体しうるため、dropされるechoがイベントとして
    // 発行され終えてから次のechoを発生させ、gap検出契機を決定的にする
    wait_surface_contains(&runtime, &owner, "received-2:fault-gap").await;
    runtime
        .write(owner.clone(), "after-gap\r")
        .expect("write after dropped production event");
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(10), attached.next())
            .await
            .expect("gap must produce a resynchronization item"),
        Some(TerminalSurfaceStreamItemV1::Snapshot { .. })
    ));
    runtime
        .write(owner.clone(), "after-gap-resync\r")
        .expect("write continuity marker after gap resynchronization");
    receive_until(&mut attached, "received-4:after-gap-resync").await;

    faults.arm(TerminalSurfaceEventFault::ReverseNextTwo);
    runtime
        .resize(owner.clone(), 24, 241)
        .expect("publish first reversed production event");
    runtime
        .write(owner.clone(), "fault-reversal\r")
        .expect("publish second reversed production event through real PTY");
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(10), attached.next())
            .await
            .expect("reversal must produce a resynchronization item"),
        Some(TerminalSurfaceStreamItemV1::Snapshot { .. })
    ));

    runtime.kill(owner).expect("kill fault-injection PTY");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_011_terminal_checkpointが画面属性と終了後のbounded_scrollbackを復元する() {
    const FRAME_COUNT: usize = 550;

    let data_dir = tempfile::TempDir::new().unwrap();
    let cwd = tempfile::TempDir::new().unwrap();
    let path = cwd.path().to_string_lossy().into_owned();
    let owner = workspace_owner(&path);
    let (_app, runtime) = build_runtime(data_dir.path());
    runtime
        .get_or_spawn(
            24,
            80,
            Some(path.clone()),
            owner.clone(),
            Some("ATUI-011 fixture".to_string()),
        )
        .expect("spawn production PTY");
    let fixture = FixturePlan {
        input_lines: 2,
        cursor_after_input: Some((0, 4, 1)),
        ..FixturePlan::new("atui-011-wide-日本語🙂", vec![])
    };
    runtime
        .write(
            owner.clone(),
            &format!("{}\n", fixture_process_shell_command(&fixture)),
        )
        .expect("launch alternate-screen fixture");
    let mut monitor = runtime
        .attach("atui-011-monitor".to_string(), owner.clone())
        .expect("attach alternate-screen monitor");
    receive_until(&mut monitor, "atui-011-wide-日本語🙂").await;
    runtime
        .resize(owner.clone(), 37, 111)
        .expect("resize Terminal Surface");
    runtime
        .write(owner.clone(), "style-probe\r")
        .expect("write styled frame");
    receive_until(&mut monitor, "received-0:style-probe").await;
    wait_surface_cursor(&runtime, &owner, (0, 3)).await;
    let mut attached = runtime
        .attach("atui-011-checkpoint".to_string(), owner.clone())
        .expect("attach checkpoint");
    let surface = snapshot(&[attached.next().await.expect("checkpoint snapshot")]);
    assert_eq!(
        (surface.terminal_surface.cols, surface.terminal_surface.rows),
        (111, 37)
    );
    let mut restored = avt::Vt::builder()
        .size(111, 37)
        .scrollback_limit(RECONSTRUCTION_SCROLLBACK_ROWS)
        .build();
    restored.feed_str(&surface.terminal_surface.replay);
    let active_lines = restored.view().collect::<Vec<_>>();
    let styled = active_lines
        .iter()
        .find_map(|line| {
            let start = line.text().find("received-0:style-probe")?;
            line.cells().get(start)
        })
        .expect("styled response cell");
    assert!(styled.pen().is_bold());
    assert_eq!(styled.pen().foreground(), Some(avt::Color::Indexed(2)));
    assert_eq!(
        active_lines
            .iter()
            .flat_map(|line| line.cells())
            .find(|cell| cell.char() == '日')
            .expect("wide Japanese cell")
            .width(),
        2
    );
    assert_eq!(restored.cursor(), (0, 3));
    runtime.kill(owner).expect("kill alternate-screen PTY");

    let bounded_owner = session_owner(&path, "bounded");
    runtime
        .get_or_spawn(
            24,
            160,
            Some(path.clone()),
            bounded_owner.clone(),
            Some("ATUI-011 bounded fixture".to_string()),
        )
        .expect("spawn bounded PTY");
    let bounded_fixture = FixturePlan {
        input_lines: FRAME_COUNT,
        alternate_screen: false,
        ..FixturePlan::new("atui-011-bounded", vec![])
    };
    runtime
        .write(
            bounded_owner.clone(),
            &format!("{}\n", fixture_process_shell_command(&bounded_fixture)),
        )
        .expect("launch bounded fixture");
    let mut bounded_monitor = runtime
        .attach(
            "atui-011-bounded-monitor".to_string(),
            bounded_owner.clone(),
        )
        .expect("attach bounded monitor");
    receive_until(&mut bounded_monitor, "atui-011-bounded").await;
    for index in 0..FRAME_COUNT {
        let marker = format!("bounded-{index:03}-{}", "x".repeat(100));
        runtime
            .write(bounded_owner.clone(), &format!("{marker}\r"))
            .expect("write bounded frame");
        receive_until(&mut bounded_monitor, &format!("received-{index}:{marker}")).await;
    }
    runtime
        .write(bounded_owner.clone(), "exit\r")
        .expect("exit bounded PTY");
    receive_exit(&mut bounded_monitor).await;
    let exited = runtime
        .get(bounded_owner)
        .expect("read exited Terminal Surface");
    assert!(exited.is_exited);
    let mut restored = avt::Vt::builder()
        .size(
            usize::from(exited.terminal_surface.cols),
            usize::from(exited.terminal_surface.rows),
        )
        .scrollback_limit(RECONSTRUCTION_SCROLLBACK_ROWS)
        .build();
    restored.feed_str(&exited.terminal_surface.replay);
    let text = restored.text().join("\n");
    assert!(!text.contains("received-0:bounded-000-"));
    assert!(text.contains("received-549:bounded-549-"));
    assert_eq!(
        restored.lines().count(),
        24 + PRODUCTION_SCROLLBACK_ROWS + 1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_011_複数terminal_surfaceの画面状態が混線しない() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let cwd = tempfile::TempDir::new().unwrap();
    let path = cwd.path().to_string_lossy().into_owned();
    let first_owner = session_owner(&path, "first");
    let second_owner = session_owner(&path, "second");
    let (_app, runtime) = build_runtime(data_dir.path());

    for (owner, label) in [
        (first_owner.clone(), "terminal-surface-first"),
        (second_owner.clone(), "terminal-surface-second"),
    ] {
        runtime
            .get_or_spawn(
                24,
                200,
                Some(path.clone()),
                owner.clone(),
                Some(label.to_string()),
            )
            .expect("spawn isolated production PTY");
    }

    let mut first = runtime
        .attach("atui-011-first".to_string(), first_owner.clone())
        .expect("attach first surface");
    let mut second = runtime
        .attach("atui-011-second".to_string(), second_owner.clone())
        .expect("attach second surface");
    let fixture_command = |label| {
        fixture_process_shell_command(&FixturePlan {
            input_lines: 1,
            alternate_screen: false,
            ..FixturePlan::new(label, vec![])
        })
    };
    runtime
        .write(
            first_owner.clone(),
            &format!("{}\n", fixture_command("terminal-surface-first")),
        )
        .expect("launch first isolated fixture");
    let mut first_items = receive_until(&mut first, "terminal-surface-first 日本語🙂").await;
    runtime
        .write(
            second_owner.clone(),
            &format!("{}\n", fixture_command("terminal-surface-second")),
        )
        .expect("launch second isolated fixture");
    let mut second_items = receive_until(&mut second, "terminal-surface-second 日本語🙂").await;

    runtime
        .write(first_owner.clone(), "only-first\r")
        .expect("write first surface");
    runtime
        .write(second_owner.clone(), "only-second\r")
        .expect("write second surface");
    first_items.extend(receive_until(&mut first, "received-0:only-first").await);
    second_items.extend(receive_until(&mut second, "received-0:only-second").await);

    let first_surface = snapshot(&first_items);
    let second_surface = snapshot(&second_items);
    let first_text = reconstruct(&first_surface, &output_events(&first_items)).unwrap();
    let second_text = reconstruct(&second_surface, &output_events(&second_items)).unwrap();
    assert!(first_text.contains("terminal-surface-first"));
    assert!(first_text.contains("received-0:only-first"));
    assert!(!first_text.contains("terminal-surface-second"));
    assert!(!first_text.contains("only-second"));
    assert!(second_text.contains("terminal-surface-second"));
    assert!(second_text.contains("received-0:only-second"));
    assert!(!second_text.contains("terminal-surface-first"));
    assert!(!second_text.contains("only-first"));

    runtime.kill(first_owner).expect("kill first PTY");
    runtime.kill(second_owner).expect("kill second PTY");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_012_app再構築後は同一process扱いせず最終画面だけをcold_restoreする() {
    const FRAME_COUNT: usize = 550;

    let data_dir = tempfile::TempDir::new().unwrap();
    let cwd = tempfile::TempDir::new().unwrap();
    let path = cwd.path().to_string_lossy().into_owned();
    let owner = workspace_owner(&path);
    let startup_count_path = cwd.path().join("startup-count");
    let first_pid_path = cwd.path().join("first-pid");
    let second_pid_path = cwd.path().join("second-pid");
    let startup_command = format!(
        "printf x >> {}; printf %s \"$$\" > {}",
        shell_quote(&startup_count_path.to_string_lossy()),
        shell_quote(&first_pid_path.to_string_lossy())
    );

    let (first_app, first_runtime) = build_runtime(data_dir.path());
    let first = first_runtime
        .get_or_spawn_with_startup(
            24,
            100,
            Some(path.clone()),
            owner.clone(),
            Some("ATUI-012 fixture".to_string()),
            Some(startup_command.clone()),
        )
        .expect("spawn first product PTY");
    assert_eq!(wait_file_content(&startup_count_path).await, "x");
    let first_pid = wait_file_content(&first_pid_path).await;
    let fixture = FixturePlan {
        input_lines: FRAME_COUNT,
        alternate_screen: false,
        ..FixturePlan::new("atui-012-cold-restore", vec![])
    };
    first_runtime
        .write(
            owner.clone(),
            &format!("{}\n", fixture_process_shell_command(&fixture)),
        )
        .expect("launch cold-restore fixture");
    let mut monitor = first_runtime
        .attach("atui-012-monitor".to_string(), owner.clone())
        .expect("attach first runtime");
    receive_until(&mut monitor, "atui-012-cold-restore").await;
    for index in 0..FRAME_COUNT {
        let marker = format!("restart-{index:03}-{}", "x".repeat(80));
        first_runtime
            .write(owner.clone(), &format!("{marker}\r"))
            .expect("write restart scrollback frame");
        receive_until(&mut monitor, &format!("received-{index}:{marker}")).await;
    }
    drop(monitor);
    first_runtime
        .shutdown()
        .expect("stop, drain, and checkpoint first app process");
    drop(first_runtime);
    drop(first_app);
    #[cfg(unix)]
    wait_process_exit(&first_pid).await;

    let (_second_app, second_runtime) = build_runtime(data_dir.path());
    let restored = second_runtime
        .get_or_spawn_with_startup(
            24,
            100,
            Some(path),
            owner.clone(),
            Some("ATUI-012 fixture".to_string()),
            Some(startup_command),
        )
        .expect("cold restore into new PTY");
    assert!(restored.is_new);
    assert!(restored.restored_from_checkpoint);
    assert!(!restored.is_exited);
    assert_eq!(restored.session_key, first.session_key);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(std::fs::read_to_string(&startup_count_path).unwrap(), "x");
    second_runtime
        .write(
            owner.clone(),
            &format!(
                "printf %s \"$$\" > {}\n",
                shell_quote(&second_pid_path.to_string_lossy())
            ),
        )
        .expect("capture restored shell pid");
    let second_pid = wait_file_content(&second_pid_path).await;
    assert_ne!(first_pid, second_pid);
    let mut attachment = second_runtime
        .attach("atui-012-restored".to_string(), owner.clone())
        .expect("attach cold-restored surface");
    let surface = snapshot(&[attachment.next().await.expect("restored snapshot")]);
    let mut terminal = avt::Vt::builder()
        .size(
            usize::from(surface.terminal_surface.cols),
            usize::from(surface.terminal_surface.rows),
        )
        .scrollback_limit(RECONSTRUCTION_SCROLLBACK_ROWS)
        .build();
    terminal.feed_str(&surface.terminal_surface.replay);
    let text = terminal.text().join("\n");
    assert!(!text.contains("received-0:restart-000-"));
    assert!(text.contains("received-549:restart-549-"));
    assert_eq!(
        terminal.lines().count(),
        24 + PRODUCTION_SCROLLBACK_ROWS + 1
    );
    second_runtime.kill(owner).expect("kill restored PTY");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_012_通常終了は実ptyを停止して出力drain後の最終画面をcold_restoreする() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let cwd = tempfile::TempDir::new().unwrap();
    let path = cwd.path().to_string_lossy().into_owned();
    let owner = session_owner(&path, "shutdown-drain");
    let (_first_app, first_runtime) = build_runtime(data_dir.path());
    first_runtime
        .get_or_spawn(
            24,
            120,
            Some(path.clone()),
            owner.clone(),
            Some("ATUI-012 shutdown drain".to_string()),
        )
        .expect("spawn shutdown-drain PTY");
    let mut monitor = first_runtime
        .attach("atui-012-shutdown-monitor".to_string(), owner.clone())
        .expect("attach shutdown-drain monitor");
    assert!(matches!(
        monitor.next().await,
        Some(TerminalSurfaceStreamItemV1::Snapshot { .. })
    ));
    first_runtime
        .write(
            owner.clone(),
            "i=0; while [ $i -lt 2000 ]; do printf 'shutdown-frame-%06d\\n' $i; i=$((i+1)); done; sleep 60\n",
        )
        .expect("start continuous output");
    receive_until(&mut monitor, "shutdown-frame-000100").await;

    first_runtime
        .shutdown()
        .expect("quiesce, stop, drain, flush");

    let stopped = first_runtime
        .get(owner.clone())
        .expect("read stopped surface");
    assert!(stopped.is_exited);
    let stopped_sequence = stopped.terminal_surface.sequence;
    let stopped_text = surface_text(&stopped);
    assert!(stopped_text.contains("shutdown-frame-"));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        first_runtime
            .get(owner.clone())
            .expect("read stable stopped surface")
            .terminal_surface
            .sequence,
        stopped_sequence
    );
    assert!(first_runtime
        .write(owner.clone(), "echo must-not-run\n")
        .is_err());
    assert!(first_runtime
        .get_or_spawn(
            24,
            80,
            Some(path.clone()),
            session_owner(&path, "must-not-spawn"),
            None,
        )
        .is_err());
    drop(first_runtime);

    let (_second_app, second_runtime) = build_runtime(data_dir.path());
    let restored = second_runtime
        .get_or_spawn(24, 120, Some(path), owner.clone(), None)
        .expect("cold restore final drained screen");
    assert!(restored.restored_from_checkpoint);
    let restored_surface = second_runtime
        .get(owner.clone())
        .expect("read restored surface");
    assert!(surface_text(&restored_surface).contains("shutdown-frame-"));
    second_runtime.kill(owner).expect("kill restored PTY");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_atui_012_明示kill後はcheckpointを復元せず起動コマンドを再実行する() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let cwd = tempfile::TempDir::new().unwrap();
    let path = cwd.path().to_string_lossy().into_owned();
    let owner = session_owner(&path, "explicit-kill");
    let startup_count = cwd.path().join("explicit-kill-startup-count");
    let startup_command = format!(
        "printf x >> {}",
        shell_quote(&startup_count.to_string_lossy())
    );
    let (_app, runtime) = build_runtime(data_dir.path());

    let first = runtime
        .get_or_spawn_with_startup(
            24,
            80,
            Some(path.clone()),
            owner.clone(),
            None,
            Some(startup_command.clone()),
        )
        .expect("spawn first explicit-kill PTY");
    assert!(!first.restored_from_checkpoint);
    wait_file_content_equals(&startup_count, "x").await;

    runtime
        .kill(owner.clone())
        .expect("stop, drain, delete checkpoint");
    let regenerated = runtime
        .get_or_spawn_with_startup(
            24,
            80,
            Some(path),
            owner.clone(),
            None,
            Some(startup_command),
        )
        .expect("regenerate after explicit kill");

    assert!(!regenerated.restored_from_checkpoint);
    wait_file_content_equals(&startup_count, "xx").await;
    runtime.kill(owner).expect("kill regenerated PTY");
}
