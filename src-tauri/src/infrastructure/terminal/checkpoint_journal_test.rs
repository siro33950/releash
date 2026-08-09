use super::IncrementalCheckpointJournal;
use crate::infrastructure::terminal::terminal_emulator::{
    NativeTerminalCheckpoint, NativeTerminalCheckpointRecord,
};

fn base() -> NativeTerminalCheckpoint {
    NativeTerminalCheckpoint {
        replay: String::new(),
        sequence: 0,
        cols: 80,
        rows: 24,
    }
}

#[test]
fn test_増分収集_連番をdrainし失敗時は同じ順序で戻す() {
    let mut journal = IncrementalCheckpointJournal::new(base(), false);
    journal
        .record(NativeTerminalCheckpointRecord::Output {
            sequence: 1,
            data: "first".into(),
        })
        .unwrap();
    journal
        .record(NativeTerminalCheckpointRecord::Resize {
            sequence: 2,
            cols: 100,
            rows: 30,
        })
        .unwrap();

    let flush = journal.take_pending();
    assert_eq!(flush.base.as_ref().map(|base| base.sequence), Some(0));
    assert_eq!(
        flush
            .records
            .iter()
            .map(NativeTerminalCheckpointRecord::sequence)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    journal.restore_failed(flush);
    let retry = journal.take_pending();
    assert_eq!(
        retry
            .records
            .iter()
            .map(NativeTerminalCheckpointRecord::sequence)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn test_増分収集_重複逆転欠落を拒否する() {
    let mut journal = IncrementalCheckpointJournal::new(base(), true);
    journal
        .record(NativeTerminalCheckpointRecord::Barrier { sequence: 1 })
        .unwrap();

    assert!(journal
        .record(NativeTerminalCheckpointRecord::Barrier { sequence: 1 })
        .is_err());
    assert!(journal
        .record(NativeTerminalCheckpointRecord::Barrier { sequence: 3 })
        .is_err());
}
