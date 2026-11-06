use super::*;

fn entry(name: &str, kind: DataEntryKind, bytes: u64) -> DataEntry {
    DataEntry {
        name: name.into(),
        kind,
        bytes,
    }
}

#[test]
fn test_保存先_既定だけが旧保存先を持つ() {
    // Given
    let current = PathBuf::from("/data/current");
    let legacy = Path::new("/data/legacy");
    // When / Then
    for (default, old, expected) in [
        (
            Some(current.as_path()),
            Some(legacy),
            Some(legacy.to_owned()),
        ),
        (Some(Path::new("/data/other")), Some(legacy), None),
        (None, Some(legacy), None),
        (Some(current.as_path()), None, None),
    ] {
        let home = DataHome::for_startup(current.clone(), default, old);
        assert_eq!(home.current, current);
        assert_eq!(home.legacy, expected);
    }
}

#[test]
fn test_保存項目_利用者定義と移行印と空の項目はデータでない() {
    // Given / When / Then
    for kind in [DataEntryKind::File, DataEntryKind::Directory] {
        for name in ["workflows", "prompts", RELOCATION_MARKER] {
            assert!(!entry(name, kind, 1).is_server_data(), "{name} {kind:?}");
        }
        assert!(!entry("data", kind, 0).is_server_data());
        assert!(entry("data", kind, 1).is_server_data());
        assert!(entry("workflows-backup", kind, 1).is_server_data());
    }
}

fn survey() -> RelocationSurvey {
    RelocationSurvey {
        legacy_entries: vec![entry("store", DataEntryKind::File, 1)],
        current_entries: vec![],
        legacy_server: LegacyServer::Stopped,
        relocation_started: false,
    }
}

#[test]
fn test_移行判断_旧データが無ければ移行不要() {
    // Given
    for entries in [
        vec![],
        vec![
            entry("workflows", DataEntryKind::Directory, 1),
            entry("empty", DataEntryKind::File, 0),
        ],
    ] {
        let observed = RelocationSurvey {
            legacy_entries: entries,
            ..survey()
        };
        // When / Then
        assert_eq!(observed.decide(), Ok(RelocationPlan::NotNeeded));
    }
}

#[test]
fn test_移行判断_旧サーバの稼働と確認不能は再開時も拒否する() {
    // Given
    for (server, block) in [
        (LegacyServer::Running, RelocationBlock::LegacyServerRunning),
        (
            LegacyServer::Unverifiable,
            RelocationBlock::LegacyServerUnverifiable,
        ),
    ] {
        for relocation_started in [false, true] {
            let observed = RelocationSurvey {
                legacy_server: server,
                relocation_started,
                ..survey()
            };
            // When / Then
            assert_eq!(observed.decide(), Err(block));
        }
    }
}

#[test]
fn test_移行判断_両方のデータを拒否するが移行印があれば再開する() {
    // Given
    let mut observed = RelocationSurvey {
        current_entries: vec![entry("other", DataEntryKind::File, 1)],
        ..survey()
    };
    // When / Then
    assert_eq!(observed.decide(), Err(RelocationBlock::BothHaveData));
    observed.relocation_started = true;
    assert_eq!(
        observed.decide(),
        Ok(RelocationPlan::Move {
            entries: vec!["store".into()]
        })
    );
}

#[test]
fn test_移行判断_停止確認後は定義を除く全項目を名前順に移す() {
    // Given
    for server in [LegacyServer::Absent, LegacyServer::Stopped] {
        let observed = RelocationSurvey {
            legacy_entries: vec![
                entry("z", DataEntryKind::File, 1),
                entry("workflows", DataEntryKind::Directory, 1),
                entry("prompts", DataEntryKind::Directory, 1),
                entry("a", DataEntryKind::Directory, 0),
            ],
            current_entries: vec![
                entry("workflows", DataEntryKind::Directory, 1),
                entry("prompts", DataEntryKind::Directory, 1),
                entry("z", DataEntryKind::File, 0),
            ],
            legacy_server: server,
            ..survey()
        };
        // When / Then
        assert_eq!(
            observed.decide(),
            Ok(RelocationPlan::Move {
                entries: vec!["a".into(), "z".into()]
            })
        );
    }
}

#[test]
fn test_旧サーバ判定_記録なしと不正な記録ではprocessを観測しない() {
    // Given
    for (record, expected) in [
        (LegacyServerRecord::Missing, LegacyServer::Absent),
        (LegacyServerRecord::Unreadable, LegacyServer::Unverifiable),
        (
            LegacyServerRecord::Recorded {
                pid: 0,
                process_started_at: 1,
            },
            LegacyServer::Unverifiable,
        ),
        (
            LegacyServerRecord::Recorded {
                pid: 1,
                process_started_at: 0,
            },
            LegacyServer::Unverifiable,
        ),
    ] {
        // When
        let actual = LegacyServer::judge(record, |_| {
            panic!("invalid record must not look up a process")
        });
        // Then
        assert_eq!(actual, expected);
    }
}

#[test]
fn test_旧サーバ判定_記録のpidを観測し開始時刻で同一性を決める() {
    // Given
    for (observed, expected) in [
        (ProcessLookup::StartedAt(1), LegacyServer::Running),
        (ProcessLookup::StartedAt(2), LegacyServer::Stopped),
        (ProcessLookup::NotFound, LegacyServer::Stopped),
        (ProcessLookup::StartedAt(0), LegacyServer::Unverifiable),
        (ProcessLookup::Unavailable, LegacyServer::Unverifiable),
    ] {
        let mut calls = vec![];
        // When
        let actual = LegacyServer::judge(
            LegacyServerRecord::Recorded {
                pid: 7,
                process_started_at: 1,
            },
            |pid| {
                calls.push(pid);
                observed
            },
        );
        // Then
        assert_eq!(actual, expected);
        assert_eq!(calls, vec![7]);
    }
}

#[test]
fn test_項目移動_移動元がなければ移動先を変更しない() {
    // Given
    for destination in [
        None,
        Some(EntryShape::File { bytes: 0 }),
        Some(EntryShape::File { bytes: 1 }),
        Some(EntryShape::Directory),
        Some(EntryShape::Other),
    ] {
        // When / Then
        assert_eq!(MoveAction::decide(None, destination), MoveAction::Skip);
    }
}

#[test]
fn test_項目移動_移動先がなければ全種類を移す() {
    // Given
    for source in [
        EntryShape::File { bytes: 0 },
        EntryShape::File { bytes: 1 },
        EntryShape::Directory,
        EntryShape::Other,
    ] {
        // When / Then
        assert_eq!(MoveAction::decide(Some(source), None), MoveAction::Rename);
    }
}

#[test]
fn test_項目移動_ディレクトリ同士と空ファイル以外は上書きを拒否する() {
    // Given
    use EntryShape::{Directory, File, Other};
    let shapes = [File { bytes: 0 }, File { bytes: 1 }, Directory, Other];
    let expected = [
        [
            MoveAction::Rename,
            MoveAction::Refuse,
            MoveAction::Refuse,
            MoveAction::Refuse,
        ],
        [
            MoveAction::Rename,
            MoveAction::Refuse,
            MoveAction::Refuse,
            MoveAction::Refuse,
        ],
        [
            MoveAction::Refuse,
            MoveAction::Refuse,
            MoveAction::MergeDirectory,
            MoveAction::Refuse,
        ],
        [MoveAction::Refuse; 4],
    ];
    for (row, source) in shapes.into_iter().enumerate() {
        for (column, destination) in shapes.into_iter().enumerate() {
            // When / Then
            assert_eq!(
                MoveAction::decide(Some(source), Some(destination)),
                expected[row][column],
                "{source:?} -> {destination:?}"
            );
        }
    }
}

#[test]
fn test_移行失敗変換_拒否理由を起動失敗へ保持する() {
    // Given
    for block in [
        RelocationBlock::LegacyServerRunning,
        RelocationBlock::LegacyServerUnverifiable,
        RelocationBlock::BothHaveData,
    ] {
        // When
        let actual = super::super::StartupFailureKind::from(RelocationError::Blocked(block));
        // Then
        assert_eq!(
            actual,
            super::super::StartupFailureKind::DataRelocationBlocked(block)
        );
    }
}

#[test]
fn test_移行失敗変換_技術的失敗の性質を起動失敗へ保持する() {
    // Given
    use crate::domain::failure::TechnicalFailureNature;
    for nature in [
        TechnicalFailureNature::Transient,
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
        TechnicalFailureNature::Other,
    ] {
        // When
        let actual =
            super::super::StartupFailureKind::from(RelocationError::Technical(TechnicalFailure {
                nature,
                message: "survey or relocation failed".into(),
            }));
        // Then
        assert_eq!(
            actual,
            super::super::StartupFailureKind::DataRelocationFailed(nature)
        );
    }
}
