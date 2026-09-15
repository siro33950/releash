use super::*;

fn identity(command: &str, value: u8, target: u8) -> OperationIdentity {
    OperationIdentity {
        command: command.into(),
        fingerprint: [value; 32],
        target: policy::ordering_scope(command).map(|scope| (scope, [target; 32])),
    }
}
fn previous(id: &str, command: &str) -> OperationReference {
    OperationReference {
        id: id.into(),
        command: command.into(),
        uncertain: true,
        fingerprint: Some([1; 32]),
        target: Some([1; 32]),
    }
}

#[test]
fn test_操作受理_同じ識別子は一度だけ実行し完了まで保持する() {
    // Given
    let mut registry = OperationRegistry::new("current".into());
    let operation = identity("append_review_comment", 1, 0);
    // When / Then
    assert_eq!(
        registry.admit(
            "operation",
            "current",
            operation.clone(),
            &[],
            &RecoveryAttempt {
                sent: false,
                ..Default::default()
            }
        ),
        OperationDecision::Ready
    );
    assert_eq!(
        registry.admit(
            "operation",
            "current",
            operation.clone(),
            &[],
            &RecoveryAttempt {
                sent: false,
                ..Default::default()
            }
        ),
        OperationDecision::Pending
    );
    assert_eq!(
        registry.admit(
            "operation",
            "current",
            identity("append_review_comment", 2, 0),
            &[],
            &RecoveryAttempt {
                sent: false,
                ..Default::default()
            }
        ),
        OperationDecision::Conflict
    );
    registry.acknowledge("operation", false);
    assert_eq!(
        registry.query("operation", "current"),
        OperationDecision::Pending
    );
    registry.complete("operation", &42, None);
    assert_eq!(
        registry.admit(
            "operation",
            "current",
            operation,
            &[],
            &RecoveryAttempt {
                sent: false,
                ..Default::default()
            }
        ),
        OperationDecision::Completed(42)
    );
    registry.acknowledge("operation", false);
    assert_eq!(
        registry.query("operation", "current"),
        OperationDecision::Unknown
    );
}

#[test]
fn test_操作受理_未確認の操作を退避せず上限で新規操作を拒否する() {
    // Given
    let mut registry = OperationRegistry::<()>::new("current".into());
    // When
    for index in 0..MAX_UNACKNOWLEDGED_OPERATIONS {
        assert_eq!(
            registry.admit(
                &index.to_string(),
                "current",
                identity("append_review_comment", 1, 0),
                &[],
                &RecoveryAttempt {
                    sent: false,
                    ..Default::default()
                }
            ),
            OperationDecision::Ready
        );
    }
    // Then
    assert_eq!(
        registry.admit(
            "overflow",
            "current",
            identity("append_review_comment", 1, 0),
            &[],
            &RecoveryAttempt {
                sent: false,
                ..Default::default()
            }
        ),
        OperationDecision::Full
    );
    assert_eq!(registry.query("0", "current"), OperationDecision::Pending);
}

#[test]
fn test_操作復旧_世代と期限に従い再実行の安全を判定する() {
    // Given
    let mut registry = OperationRegistry::<()>::new("new".into());
    // When / Then
    for (command, recoverable) in [
        ("add_repo_path", false),
        ("request_application_quit", true),
        ("start_workflow", false),
        ("stop_watching", false),
        ("create_agent_session", false),
    ] {
        let operation = identity(command, 1, 1);
        assert_eq!(
            registry.admit(
                command,
                "old",
                operation.clone(),
                &[],
                &RecoveryAttempt {
                    sent: false,
                    ..Default::default()
                }
            ),
            OperationDecision::Unknown
        );
        assert_eq!(
            registry.prepare(
                command,
                "old",
                &operation,
                &[],
                &RecoveryAttempt {
                    sent: true,
                    expired: false,
                    ..Default::default()
                }
            ),
            if recoverable {
                OperationDecision::Ready
            } else {
                OperationDecision::Unknown
            }
        );
        assert_eq!(
            registry.admit(
                command,
                "old",
                operation,
                &[],
                &RecoveryAttempt {
                    sent: true,
                    ..Default::default()
                }
            ),
            if recoverable {
                OperationDecision::Ready
            } else {
                OperationDecision::Unknown
            }
        );
    }
    assert_eq!(
        registry.prepare(
            "expired",
            "old",
            &identity("add_repo_path", 2, 1),
            &[],
            &RecoveryAttempt {
                sent: true,
                expired: true,
                ..Default::default()
            }
        ),
        OperationDecision::Unknown
    );
    assert_eq!(
        registry.prepare(
            "retry",
            "old",
            &identity("add_repo_path", 2, 1),
            &[],
            &RecoveryAttempt {
                sent: true,
                expired: false,
                ..Default::default()
            }
        ),
        OperationDecision::Unknown
    );
}

#[test]
fn test_操作同一性_不明な元の操作に対応付け別の引数は独立させる() {
    // Given
    let mut registry = OperationRegistry::<()>::new("current".into());
    registry.admit(
        "original",
        "current",
        identity("create_agent_session", 1, 0),
        &[],
        &RecoveryAttempt {
            sent: false,
            ..Default::default()
        },
    );
    let references = [previous("original", "create_agent_session")];
    // When / Then
    assert_eq!(
        registry.prepare(
            "retry",
            "current",
            &identity("create_agent_session", 1, 0),
            &references,
            &RecoveryAttempt {
                sent: false,
                expired: false,
                ..Default::default()
            }
        ),
        OperationDecision::Bound("original".into())
    );
    assert_eq!(
        registry.prepare(
            "new",
            "current",
            &identity("create_agent_session", 2, 0),
            &references,
            &RecoveryAttempt {
                sent: false,
                expired: false,
                ..Default::default()
            }
        ),
        OperationDecision::Ready
    );
    let restarted = OperationRegistry::<()>::new("new".into());
    assert_eq!(
        restarted.prepare(
            "retry",
            "new",
            &identity("create_agent_session", 1, 0),
            &references,
            &RecoveryAttempt {
                sent: false,
                expired: false,
                ..Default::default()
            }
        ),
        OperationDecision::Bound("original".into())
    );
    assert_eq!(
        restarted.prepare(
            "different",
            "new",
            &identity("create_agent_session", 2, 0),
            &references,
            &RecoveryAttempt {
                sent: false,
                expired: false,
                ..Default::default()
            }
        ),
        OperationDecision::Ready
    );
}

#[test]
fn test_操作順序_異なるcommandでも同じ対象の復旧を追い越さない() {
    // Given
    let mut registry = OperationRegistry::new("new".into());
    let references = [previous("add", "add_repo_path")];
    let remove = identity("remove_repo_path", 2, 1);
    // When / Then
    assert_eq!(
        registry.prepare(
            "remove",
            "new",
            &remove,
            &references,
            &RecoveryAttempt {
                sent: false,
                expired: false,
                ..Default::default()
            }
        ),
        OperationDecision::Blocked
    );
    assert_eq!(
        registry.admit(
            "add",
            "new",
            identity("add_repo_path", 1, 1),
            &[],
            &RecoveryAttempt {
                sent: true,
                ..Default::default()
            }
        ),
        OperationDecision::Ready
    );
    assert_eq!(
        registry.prepare(
            "remove",
            "new",
            &remove,
            &references,
            &RecoveryAttempt {
                sent: false,
                expired: false,
                ..Default::default()
            }
        ),
        OperationDecision::Blocked
    );
    assert_eq!(
        registry.prepare(
            "other",
            "new",
            &identity("remove_repo_path", 3, 2),
            &references,
            &RecoveryAttempt {
                sent: false,
                expired: false,
                ..Default::default()
            }
        ),
        OperationDecision::Ready
    );
    registry.complete("add", &(), None);
    assert_eq!(
        registry.query("add", "new"),
        OperationDecision::Completed(())
    );
    assert_eq!(
        registry.prepare(
            "remove",
            "new",
            &remove,
            &references,
            &RecoveryAttempt {
                sent: false,
                expired: false,
                ..Default::default()
            }
        ),
        OperationDecision::Ready
    );
}

#[test]
fn test_監視確定_受領確認後は結果を保持し停止で解放する() {
    // Given
    let mut registry = OperationRegistry::new("current".into());
    // When / Then
    registry.admit(
        "watch",
        "current",
        identity("start_watching", 1, 0),
        &[],
        &RecoveryAttempt {
            connection: "socket",
            ..Default::default()
        },
    );
    registry.complete("watch", &"watch-result", Some(42));
    registry.acknowledge("watch", false);
    assert_eq!(
        registry.query("watch", "current"),
        OperationDecision::Completed("watch-result")
    );
    assert_eq!(registry.query("watch", "other"), OperationDecision::Unknown);
    assert_eq!(
        registry.prepare(
            "watch",
            "current",
            &identity("start_watching", 2, 0),
            &[],
            &RecoveryAttempt::default()
        ),
        OperationDecision::Conflict
    );
    registry.forget_watch(42);
    assert_eq!(
        registry.query("watch", "current"),
        OperationDecision::Unknown
    );
}

#[test]
fn test_保持期間_完了から五分保持し破棄後の自動再実行を止める() {
    // Given
    for command in [
        "write_terminal_surface",
        "add_repo_path",
        "create_agent_session",
    ] {
        let mut registry = OperationRegistry::new("current".into());
        let operation = identity(command, 1, 1);
        registry.admit(
            "op",
            "current",
            operation.clone(),
            &[],
            &RecoveryAttempt::default(),
        );
        registry.advance(50_000);
        registry.complete("op", &42, None);
        // When / Then
        registry.advance(50_000 + 120_000);
        assert_eq!(
            registry.query("op", "current"),
            OperationDecision::Completed(42)
        );
        registry.advance(50_000 + policy::OPERATION_RETENTION_MS);
        assert_eq!(registry.query("op", "current"), OperationDecision::Unknown);
        for retry in [false, true] {
            let attempt = RecoveryAttempt {
                sent: true,
                user_retry: retry,
                ..Default::default()
            };
            assert_eq!(
                registry.prepare("op", "current", &operation, &[], &attempt),
                if retry && command == "create_agent_session" {
                    OperationDecision::Ready
                } else {
                    OperationDecision::Unknown
                }
            );
        }
    }
}

#[test]
fn test_保持上限_完了済みの古い順に破棄し処理中を保持する() {
    // Given
    let mut registry = OperationRegistry::new("current".into());
    for index in 0..MAX_UNACKNOWLEDGED_OPERATIONS {
        registry.admit(
            &index.to_string(),
            "current",
            identity("write_terminal_surface", 1, 0),
            &[],
            &RecoveryAttempt::default(),
        );
    }
    registry.advance(1);
    registry.complete("2", &2, None);
    registry.complete("1", &1, None);
    // When / Then
    for evicted in ["2", "1"] {
        assert_eq!(
            registry.admit(
                &format!("new-{evicted}"),
                "current",
                identity("write_terminal_surface", 1, 0),
                &[],
                &RecoveryAttempt::default()
            ),
            OperationDecision::Ready
        );
        assert_eq!(
            registry.query(evicted, "current"),
            OperationDecision::Unknown
        );
    }
    registry.advance(policy::OPERATION_RETENTION_MS + 1);
    assert_eq!(registry.query("0", "current"), OperationDecision::Pending);
    assert_eq!(
        registry.admit(
            "full",
            "current",
            identity("write_terminal_surface", 1, 0),
            &[],
            &RecoveryAttempt::default()
        ),
        OperationDecision::Full
    );
}

#[test]
fn test_未受理の期限超過_自動は照会のみで利用者再試行だけを一度受け付ける() {
    // Given
    for (command, generation) in [
        ("create_agent_session", "current"),
        ("append_review_comment", "current"),
        ("request_application_quit", "old"),
    ] {
        let mut registry = OperationRegistry::<()>::new("current".into());
        let operation = identity(command, 1, 0);
        let mut attempt = RecoveryAttempt {
            sent: true,
            expired: true,
            ..Default::default()
        };
        // When / Then
        assert_eq!(
            registry.prepare("op", generation, &operation, &[], &attempt),
            OperationDecision::Unknown
        );
        assert_eq!(
            registry.admit("op", generation, operation.clone(), &[], &attempt),
            OperationDecision::Unknown
        );
        attempt.expired = false;
        attempt.user_retry = true;
        assert_eq!(
            registry.admit("op", generation, operation.clone(), &[], &attempt),
            OperationDecision::Ready
        );
        assert_eq!(
            registry.admit("op", generation, operation, &[], &attempt),
            OperationDecision::Pending
        );
    }
}

#[test]
fn test_監視開始_照会と遅延応答の交差でも同じ監視だけを確定する() {
    // Given
    let mut registry = OperationRegistry::new("current".into());
    let operation = identity("start_watching", 1, 0);
    let mut attempt = RecoveryAttempt {
        connection: "socket",
        ..Default::default()
    };
    assert_eq!(
        registry.admit("op", "current", operation.clone(), &[], &attempt),
        OperationDecision::Ready
    );
    // When / Then
    attempt.sent = true;
    attempt.expired = true;
    assert_eq!(
        registry.prepare("op", "current", &operation, &[], &attempt),
        OperationDecision::Pending
    );
    assert_eq!(registry.complete("op", &42, Some(42)), None);
    assert_eq!(
        registry.prepare("op", "current", &operation, &[], &attempt),
        OperationDecision::Completed(42)
    );
    registry.acknowledge("op", false);
    assert!(registry.disconnect("socket").is_empty());
    assert_eq!(
        registry.prepare("op", "current", &operation, &[], &attempt),
        OperationDecision::Completed(42)
    );
}

#[test]
fn test_監視の未受領_切断と保持期限で停止し処理中の切断も完了時に回収する() {
    // Given / When / Then
    for completed in [false, true] {
        let mut registry = OperationRegistry::new("current".into());
        registry.admit(
            "op",
            "current",
            identity("start_watching", 1, 0),
            &[],
            &RecoveryAttempt {
                connection: "socket",
                ..Default::default()
            },
        );
        if completed {
            registry.complete("op", &42, Some(42));
        }
        assert_eq!(
            registry.disconnect("socket"),
            if completed { vec![42] } else { vec![] }
        );
        if !completed {
            assert_eq!(registry.complete("op", &42, Some(42)), Some(42));
        }
        assert_eq!(registry.query("op", "current"), OperationDecision::Unknown);
    }
    let mut registry = OperationRegistry::new("current".into());
    registry.admit(
        "op",
        "current",
        identity("start_watching", 1, 0),
        &[],
        &RecoveryAttempt::default(),
    );
    registry.complete("op", &42, Some(42));
    registry.advance(policy::OPERATION_RETENTION_MS);
    assert_eq!(registry.take_released_watches(), vec![42]);
    assert_eq!(registry.query("op", "current"), OperationDecision::Unknown);
}

#[test]
fn test_別世代の復旧_確定した後続削除を先行追加で取り消さない() {
    // Given
    let registry = OperationRegistry::<()>::new("new".into());
    let successors = [previous("remove", "remove_repo_path")];
    let attempt = RecoveryAttempt {
        sent: true,
        successors: &successors,
        ..Default::default()
    };
    // When / Then
    assert_eq!(
        registry.prepare(
            "add",
            "old",
            &identity("add_repo_path", 1, 1),
            &[],
            &attempt
        ),
        OperationDecision::Unknown
    );
    for generation in ["old", "new"] {
        assert_eq!(
            registry.prepare(
                "other",
                generation,
                &identity("add_repo_path", 1, 2),
                &[],
                &attempt
            ),
            if generation == "new" {
                OperationDecision::Ready
            } else {
                OperationDecision::Unknown
            }
        );
    }
}

#[test]
fn test_未受理の期限超過_届いた初回packetも実行せず結果不明にする() {
    let mut registry = OperationRegistry::<()>::new("current".into());
    let operation = identity("create_agent_session", 1, 0);
    let expired = RecoveryAttempt {
        expired: true,
        ..Default::default()
    };
    assert_eq!(
        registry.prepare("op", "current", &operation, &[], &expired),
        OperationDecision::NotSent
    );
    assert_eq!(
        registry.admit("op", "current", operation, &[], &expired),
        OperationDecision::Unknown
    );
    assert_eq!(registry.query("op", "current"), OperationDecision::Unknown);
}

#[test]
fn test_操作復旧_送信済みreadの期限超過と再送不可の接続操作は切断と判定する() {
    // Given
    let registry = OperationRegistry::<()>::new("current".into());
    use OperationDecision::{Disconnected, NotSent, Ready, Unknown};
    // When / Then
    for (command, generation, sent, expired, expected) in [
        ("get_current_branch", "current", false, true, NotSent),
        ("get_current_branch", "current", true, false, Ready),
        ("get_current_branch", "current", true, true, Disconnected),
        ("get_current_branch", "old", true, false, Ready),
        ("get_current_branch", "old", true, true, Disconnected),
        ("attach_terminal_surface", "current", false, false, Ready),
        (
            "attach_terminal_surface",
            "current",
            true,
            false,
            Disconnected,
        ),
        (
            "detach_terminal_surface",
            "current",
            true,
            false,
            Disconnected,
        ),
        ("attach_terminal_surface", "old", false, false, Disconnected),
        ("attach_terminal_surface", "old", true, false, Disconnected),
        ("attach_terminal_surface", "current", false, true, NotSent),
        ("attach_terminal_surface", "current", true, true, Unknown),
    ] {
        assert_eq!(
            registry.prepare(
                "op",
                generation,
                &identity(command, 1, 0),
                &[],
                &RecoveryAttempt {
                    sent,
                    expired,
                    ..Default::default()
                }
            ),
            expected,
            "{command}, generation={generation}, sent={sent}, expired={expired}"
        );
    }
}

#[test]
fn test_保持上限_破棄済み操作は重複を防げる利用者再試行だけ再受理する() {
    // Given
    for command in [
        "write_terminal_surface",
        "add_repo_path",
        "create_agent_session",
    ] {
        let mut registry = OperationRegistry::new("current".into());
        let operation = identity(command, 1, 1);
        registry.admit(
            "op",
            "current",
            operation.clone(),
            &[],
            &RecoveryAttempt::default(),
        );
        registry.complete("op", &42, None);
        for index in 0..MAX_UNACKNOWLEDGED_OPERATIONS {
            assert_eq!(
                registry.admit(
                    &index.to_string(),
                    "current",
                    identity("write_terminal_surface", 1, 0),
                    &[],
                    &RecoveryAttempt::default()
                ),
                OperationDecision::Ready
            );
        }
        registry.complete("0", &0, None);
        registry.acknowledge("0", false);
        // When / Then
        assert_eq!(registry.query("op", "current"), OperationDecision::Unknown);
        for user_retry in [false, true] {
            let attempt = RecoveryAttempt {
                sent: true,
                user_retry,
                ..Default::default()
            };
            let expected = if user_retry && command == "create_agent_session" {
                OperationDecision::Ready
            } else {
                OperationDecision::Unknown
            };
            assert_eq!(
                registry.prepare("op", "current", &operation, &[], &attempt),
                expected
            );
            assert_eq!(
                registry.admit("op", "current", operation.clone(), &[], &attempt),
                expected
            );
        }
        if command == "create_agent_session" {
            let retry = RecoveryAttempt {
                sent: true,
                user_retry: true,
                ..Default::default()
            };
            assert_eq!(
                registry.admit("op", "current", operation.clone(), &[], &retry),
                OperationDecision::Pending
            );
            registry.complete("op", &42, None);
            assert_eq!(
                registry.admit("op", "current", operation, &[], &retry),
                OperationDecision::Completed(42)
            );
        }
    }
}

#[test]
fn test_記録破棄_無関係な未受理要求だけを元の操作として再試行できる() {
    // Given
    let mut registry = OperationRegistry::new("current".into());
    registry.admit(
        "discarded",
        "current",
        identity("add_repo_path", 1, 1),
        &[],
        &RecoveryAttempt::default(),
    );
    registry.complete("discarded", &true, None);
    registry.advance(policy::OPERATION_RETENTION_MS);
    let retry = RecoveryAttempt {
        sent: true,
        user_retry: true,
        ..Default::default()
    };
    // When / Then
    assert_eq!(
        registry.prepare(
            "discarded",
            "current",
            &identity("add_repo_path", 1, 1),
            &[],
            &retry
        ),
        OperationDecision::Unknown
    );
    let unaccepted = identity("add_repo_path", 2, 2);
    assert_eq!(
        registry.prepare(
            "unaccepted",
            "current",
            &unaccepted,
            &[],
            &RecoveryAttempt {
                expired: true,
                ..retry
            }
        ),
        OperationDecision::Unknown
    );
    assert_eq!(
        registry.admit("unaccepted", "current", unaccepted.clone(), &[], &retry),
        OperationDecision::Ready
    );
    assert_eq!(
        registry.admit("unaccepted", "current", unaccepted, &[], &retry),
        OperationDecision::Pending
    );
    registry.complete("unaccepted", &true, None);
    registry.acknowledge("unaccepted", false);
    assert_eq!(
        registry.admit(
            "next",
            "current",
            identity("remove_repo_path", 3, 2),
            &[],
            &RecoveryAttempt::default()
        ),
        OperationDecision::Ready
    );
}

#[test]
fn test_監視復旧_期限後は元の監視を再実行せず復旧が必要と伝える() {
    // Given
    let registry = OperationRegistry::<()>::new("current".into());
    // When / Then
    for command in ["start_watching", "start_git_dir_watching"] {
        assert_eq!(
            registry.prepare(
                "expired",
                "current",
                &identity(command, 1, 1),
                &[],
                &RecoveryAttempt {
                    sent: true,
                    expired: true,
                    ..Default::default()
                }
            ),
            OperationDecision::WatchReleased
        );
    }
}

#[test]
fn test_監視復旧_解放された元の未確定監視へ新しい購読を束縛しない() {
    // Given
    let mut registry = OperationRegistry::new("current".into());
    let operation = identity("start_watching", 1, 1);
    registry.admit(
        "old",
        "current",
        operation.clone(),
        &[],
        &RecoveryAttempt {
            connection: "socket",
            ..Default::default()
        },
    );
    registry.complete("old", &42, Some(42));
    registry.disconnect("socket");
    // When / Then
    assert_eq!(
        registry.admit(
            "replacement",
            "current",
            operation,
            &[previous("old", "start_watching")],
            &RecoveryAttempt {
                connection: "reconnected",
                ..Default::default()
            }
        ),
        OperationDecision::Ready
    );
}

#[test]
fn test_監視受領_確認済みの監視も利用側の破棄通知で回収できる() {
    // Given
    let mut registry = OperationRegistry::new("current".into());
    registry.admit(
        "watch",
        "current",
        identity("start_watching", 1, 1),
        &[],
        &RecoveryAttempt {
            connection: "socket",
            ..Default::default()
        },
    );
    registry.complete("watch", &42, Some(42));
    registry.acknowledge("watch", false);
    // When / Then
    assert_eq!(registry.acknowledge("watch", true), Some(42));
    assert!(!registry.watch_active("watch"));
    assert_eq!(registry.acknowledge("watch", true), None);
}

#[test]
fn test_破棄済み冪等操作_世代切替後も自動照会と利用者再試行で再受理しない() {
    // Given
    for command in [
        "add_repo_path",
        "save_workspace_state",
        "update_external_editor",
    ] {
        for capacity in [false, true] {
            let mut old = OperationRegistry::new("old".into());
            let operation = identity(command, 1, 1);
            old.admit(
                "op",
                "old",
                operation.clone(),
                &[],
                &RecoveryAttempt::default(),
            );
            old.complete("op", &42, None);
            if capacity {
                for index in 0..MAX_UNACKNOWLEDGED_OPERATIONS {
                    old.admit(
                        &index.to_string(),
                        "old",
                        identity("write_terminal_surface", 1, 0),
                        &[],
                        &RecoveryAttempt::default(),
                    );
                }
            } else {
                old.advance(policy::OPERATION_RETENTION_MS);
            }
            assert_eq!(old.query("op", "old"), OperationDecision::Unknown);
            let mut restarted = OperationRegistry::<i32>::new("new".into());
            // When / Then
            for user_retry in [false, true] {
                let attempt = RecoveryAttempt {
                    sent: true,
                    user_retry,
                    ..Default::default()
                };
                assert_eq!(
                    restarted.prepare("op", "old", &operation, &[], &attempt),
                    OperationDecision::Unknown,
                    "{command}, capacity={capacity}, retry={user_retry}"
                );
                assert_eq!(
                    restarted.admit("op", "old", operation.clone(), &[], &attempt),
                    OperationDecision::Unknown
                );
                assert_eq!(restarted.query("op", "old"), OperationDecision::Unknown);
            }
        }
    }
}
