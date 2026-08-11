use super::{TerminalSurface, TerminalSurfaceNotWritable};
use crate::domain::terminal_surface::{
    TerminalRuntimeGeneration, TerminalSurfaceCheckpoint, TerminalSurfaceOwner,
};
use crate::domain::workspace_tree::WorkspaceIdentity;

fn surface() -> TerminalSurface {
    TerminalSurface::new(
        TerminalRuntimeGeneration::new(7),
        TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap(),
        None,
    )
}

#[test]
fn test_ターミナル画面更新_古い実行環境は置換後画面を変更できない() {
    let mut surface = surface();
    let applied = surface.apply_checkpoint(
        TerminalRuntimeGeneration::new(6),
        TerminalSurfaceCheckpoint {
            replay: "stale".to_string(),
            sequence: 1,
            cols: 80,
            rows: 24,
        },
    );

    assert!(!applied);
    assert_eq!(surface.checkpoint.sequence, 0);
}

#[test]
fn test_ターミナル画面終了_古い実行環境は置換後画面を終了できない() {
    let mut surface = surface();

    assert!(surface
        .mark_exited(TerminalRuntimeGeneration::new(6), Some(9))
        .is_none());
    assert!(!surface.process_state.is_exited());
    assert_eq!(
        surface.mark_exited(TerminalRuntimeGeneration::new(7), Some(0)),
        Some(1)
    );
    assert!(surface.process_state.is_exited());
    assert_eq!(surface.process_state.exit_code(), Some(0));
}

#[test]
fn test_ターミナル画面_復元点_古い連番を拒否する() {
    let mut surface = surface();
    assert!(surface.apply_checkpoint(
        TerminalRuntimeGeneration::new(7),
        TerminalSurfaceCheckpoint {
            replay: "new".to_string(),
            sequence: 2,
            cols: 80,
            rows: 24,
        },
    ));

    assert!(!surface.apply_checkpoint(
        TerminalRuntimeGeneration::new(7),
        TerminalSurfaceCheckpoint {
            replay: "old".to_string(),
            sequence: 1,
            cols: 80,
            rows: 24,
        },
    ));
    assert_eq!(surface.checkpoint.replay, "new");
}

#[test]
fn test_ターミナル画面_連番_出力寸法変更終了で単調増加する() {
    let mut surface = surface();
    let runtime_generation = TerminalRuntimeGeneration::new(7);

    assert_eq!(
        surface.record_output(runtime_generation, std::time::Instant::now()),
        Some(1)
    );
    assert_eq!(surface.record_resize(runtime_generation), Some(2));
    assert_eq!(surface.mark_exited(runtime_generation, Some(0)), Some(3));
    assert_eq!(surface.latest_sequence(), 3);
    assert_eq!(surface.checkpoint.sequence, 0);
    assert_eq!(
        surface.record_output(runtime_generation, std::time::Instant::now()),
        None
    );
    assert_eq!(surface.record_resize(runtime_generation), None);
    assert_eq!(surface.mark_exited(runtime_generation, Some(0)), None);
}

#[test]
fn test_ターミナル書込可否_実行中は書込を受理する() {
    let surface = surface();

    assert_eq!(surface.ensure_writable(), Ok(()));
}

#[test]
fn test_ターミナル書込可否_終了済みは書込を拒否する() {
    let mut surface = surface();
    surface
        .mark_exited(TerminalRuntimeGeneration::new(7), Some(0))
        .unwrap();

    assert_eq!(surface.ensure_writable(), Err(TerminalSurfaceNotWritable));
}

#[test]
fn test_ターミナル画面_出力記録_最終出力時刻をsummaryへ載せる() {
    let mut surface = surface();
    let runtime_generation = TerminalRuntimeGeneration::new(7);
    assert_eq!(surface.summary().last_output_at, None);

    let now = std::time::Instant::now();
    surface.record_output(runtime_generation, now).unwrap();

    assert_eq!(surface.summary().last_output_at, Some(now));

    surface.record_resize(runtime_generation).unwrap();
    assert_eq!(
        surface.summary().last_output_at,
        Some(now),
        "resizeは出力recencyを更新しない"
    );
}
