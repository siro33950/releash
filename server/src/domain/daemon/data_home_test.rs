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
