use super::*;

fn observe(
    status: &mut TerminalShellStatus,
    osc7: Option<&str>,
    updated: bool,
    process: &str,
) -> String {
    status.observe(osc7.map(str::to_owned), updated, process.into(), false);
    status.latest().unwrap().cwd.unwrap()
}

#[test]
fn test_作業ディレクトリ_osc7を優先し更新停止後はプロセスの移動へ追従する() {
    // Given
    let mut status = TerminalShellStatus::default();
    // When / Then
    assert_eq!(observe(&mut status, None, false, "/tmp"), "/tmp");
    assert_eq!(observe(&mut status, Some("/tmp/"), true, "/tmp"), "/tmp/");
    assert_eq!(observe(&mut status, Some("/tmp/"), false, "/tmp"), "/tmp/");
    assert_eq!(observe(&mut status, Some("/tmp/"), false, "/"), "/");
    assert_eq!(observe(&mut status, Some("/tmp/"), false, "/"), "/");
    assert_eq!(
        observe(&mut status, Some("/next/"), true, "/next"),
        "/next/"
    );
    assert_eq!(observe(&mut status, None, true, "/next"), "/next");
}

#[test]
fn test_shell状態_同じ状態は変化として返さず最後の状態を保持する() {
    // Given
    let mut status = TerminalShellStatus::default();
    assert_eq!(status.latest(), None);
    // When
    let first = status.observe(None, false, "/tmp".into(), true);
    let same = status.observe(None, false, "/tmp".into(), true);
    // Then
    assert_eq!(
        first,
        Some(TerminalShellState {
            cwd: Some("/tmp".into()),
            has_foreground_process: true
        })
    );
    assert_eq!(same, None);
    assert_eq!(status.latest(), first);
}

#[test]
fn test_shell終了_最後のcwdを保ち前面処理を無しにする() {
    // Given
    let mut status = TerminalShellStatus::default();
    status.observe(Some("/osc7".into()), true, "/process".into(), true);
    // When
    let exited = status.mark_exited();
    let observed = status.observe(Some("/ignored".into()), true, "/other".into(), true);
    // Then
    assert_eq!(observed, None);
    assert_eq!(status.latest(), Some(exited));
    assert_eq!(
        status.latest(),
        Some(TerminalShellState {
            cwd: Some("/osc7".into()),
            has_foreground_process: false,
        })
    );
}

#[test]
fn test_shell終了_観測前でも前面処理無しの状態を返す() {
    // Given
    let mut status = TerminalShellStatus::default();
    // When
    let exited = status.mark_exited();
    let observed = status.observe(Some("/ignored".into()), true, "/other".into(), true);
    // Then
    assert_eq!(observed, None);
    assert_eq!(status.latest(), Some(exited));
    assert_eq!(
        status.latest(),
        Some(TerminalShellState {
            cwd: None,
            has_foreground_process: false,
        })
    );
}
