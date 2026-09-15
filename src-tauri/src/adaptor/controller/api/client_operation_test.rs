use super::*;
use serde_json::json;

#[tokio::test(start_paused = true)]
async fn test_保持期限_全接続の終了後も通信なしで完了記録を破棄する() {
    use crate::domain::client_operation::registry::RecoveryAttempt;
    use crate::usecase::client_operation::OperationCompletion;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    };
    // Given
    let now = Arc::new(AtomicU64::new(0));
    let operations = Arc::new(ClientOperationUsecase::new(
        "current".into(),
        {
            let now = now.clone();
            Arc::new(move || now.load(Ordering::SeqCst))
        },
        Arc::new(|_| Box::pin(async {})),
    ));
    let result = Arc::new(());
    let retained = Arc::downgrade(&result);
    drop(
        operations
            .execute(
                "completed".into(),
                "current",
                OperationIdentity {
                    command: "add_repo_path".into(),
                    fingerprint: [1; 32],
                    target: None,
                },
                &[],
                &RecoveryAttempt {
                    connection: "socket",
                    ..Default::default()
                },
                || {
                    std::future::ready(OperationCompletion {
                        result,
                        started_watch: None,
                        stopped_watch: None,
                    })
                },
            )
            .await,
    );
    let owner = Arc::downgrade(&operations);
    let task = tokio::spawn(maintain(owner.clone()));
    operations.disconnect("socket").await;
    tokio::task::yield_now().await;
    // When / Then
    now.store(policy::OPERATION_RETENTION_MS - 1, Ordering::SeqCst);
    tokio::time::advance(Duration::from_millis(policy::TICK_INTERVAL_MS)).await;
    tokio::task::yield_now().await;
    assert!(retained.upgrade().is_some());
    now.store(policy::OPERATION_RETENTION_MS, Ordering::SeqCst);
    tokio::time::advance(Duration::from_millis(policy::TICK_INTERVAL_MS)).await;
    tokio::task::yield_now().await;
    assert!(retained.upgrade().is_none());
    drop(operations);
    assert!(owner.upgrade().is_none());
    tokio::time::advance(Duration::from_millis(policy::TICK_INTERVAL_MS)).await;
    task.await.unwrap();
}

#[test]
fn test_操作同一性_callerの試行識別子を除き引数と対象を変換する() {
    // Given
    let mut request = wire::CommandRequest::from_value("create_agent_session", json!({"workspaceIdentity":"/repo","worktreePath":"/repo","provider":"claude","rows":24,"cols":80,"callerRequestId":"first"})).unwrap();
    let first = identity(&request).unwrap();
    // When
    if let Some(wire::command_request::Command::CreateAgentSession(args)) = &mut request.command {
        args.caller_request_id = Some("retry".into());
    }
    // Then
    assert_eq!(identity(&request).unwrap(), first);
    let add = identity(
        &wire::CommandRequest::from_value("add_repo_path", json!({"path":"/repo"})).unwrap(),
    )
    .unwrap();
    let remove = identity(
        &wire::CommandRequest::from_value("remove_repo_path", json!({"path":"/repo"})).unwrap(),
    )
    .unwrap();
    assert_eq!(add.target, remove.target);
    assert_ne!(add.fingerprint, remove.fingerprint);
}

#[test]
fn test_操作状態_受理結果を転送形式へ変換する() {
    // Given / When / Then
    for (state, expected) in [
        (OperationDecision::Ready, "ready"),
        (OperationDecision::Pending, "pending"),
        (OperationDecision::Unknown, "unknown"),
        (OperationDecision::Blocked, "blocked"),
        (OperationDecision::Bound("original".into()), "bound"),
    ] {
        assert!(
            matches!(response("request", state).body, Some(Body::OperationStatus(status)) if status.state == expected)
        );
    }
    for state in [OperationDecision::Conflict, OperationDecision::Full] {
        assert!(matches!(
            response("request", state).body,
            Some(Body::Response(_))
        ));
    }
}

#[test]
fn test_操作参照_境界の識別情報を検証してdomainへ渡す() {
    // Given
    let mut request =
        wire::CommandRequest::from_value("add_repo_path", json!({"path":"/repo"})).unwrap();
    request.predecessors.push(wire::OperationReference {
        fingerprint: vec![1; 31],
        ..Default::default()
    });
    // When / Then
    assert!(input(&request).is_err());
    request.predecessors[0].fingerprint = vec![1; 32];
    assert_eq!(input(&request).unwrap().1[0].fingerprint, Some([1; 32]));
    let identity = identity(&request).unwrap();
    let Some(Body::OperationStatus(status)) =
        decision("id", OperationDecision::Ready, &identity).body
    else {
        panic!("decision")
    };
    assert_eq!(status.fingerprint, identity.fingerprint);
    assert_eq!(status.ordering_target, identity.target.unwrap().1);
}

#[test]
fn test_終了操作の同一性_相関idとcallerのidだけを除きintentを含める() {
    // Given
    let mut request = wire::CommandRequest::from_value(
        "request_application_quit",
        json!({"request":{"request_id":"quit-1","intent":{"type":"exit","code":0}}}),
    )
    .unwrap();
    let original = identity(&request).unwrap();
    // When
    request.request_id = "new-correlation".into();
    let mut retried = wire::CommandRequest::from_value(
        "request_application_quit",
        json!({"request":{"request_id":"quit-2","intent":{"type":"exit","code":0}}}),
    )
    .unwrap();
    retried.request_id = request.request_id;
    // Then
    assert_eq!(identity(&retried).unwrap(), original);
    let changed = wire::CommandRequest::from_value(
        "request_application_quit",
        json!({"request":{"request_id":"quit-2","intent":{"type":"exit","code":1}}}),
    )
    .unwrap();
    assert_ne!(
        identity(&changed).unwrap().fingerprint,
        original.fingerprint
    );
}

#[test]
fn test_操作対象_異なる範囲の空引数でも識別子が衝突しない() {
    // Given / When
    let identities: Vec<_> = [
        ("update_crash_reporting", json!({"enabled":true})),
        ("report_mounted_xterm_count", json!({"count":1})),
        ("update_performance_telemetry", json!({"enabled":true})),
    ]
    .into_iter()
    .map(|(command, args)| {
        identity(&wire::CommandRequest::from_value(command, args).unwrap()).unwrap()
    })
    .collect();
    // Then
    for (index, identity) in identities.iter().enumerate() {
        for other in &identities[index + 1..] {
            assert_ne!(identity.target.unwrap().1, other.target.unwrap().1);
        }
    }
}

#[test]
fn test_確定した設定_無関係な件数報告後の別世代復旧でも元の操作順の最終状態を守る() {
    use crate::domain::client_operation::registry::{OperationRegistry, RecoveryAttempt};
    // Given
    let commands = [
        ("first", "update_crash_reporting", json!({"enabled":true})),
        ("later", "update_crash_reporting", json!({"enabled":false})),
        (
            "unrelated",
            "report_mounted_xterm_count",
            json!({"count":3}),
        ),
    ];
    let mut registry = OperationRegistry::new("old".into());
    let mut state = (false, 0);
    let mut effects = Vec::new();
    let mut identities = Vec::new();
    for (id, command, args) in commands {
        let operation =
            identity(&wire::CommandRequest::from_value(command, args.clone()).unwrap()).unwrap();
        assert_eq!(
            registry.admit(
                id,
                "old",
                operation.clone(),
                &[],
                &RecoveryAttempt::default()
            ),
            OperationDecision::Ready
        );
        if command == "update_crash_reporting" {
            state.0 = args["enabled"].as_bool().unwrap();
        } else {
            state.1 = args["count"].as_u64().unwrap();
        }
        effects.push(id);
        registry.complete(id, &(), None);
        if id != "first" {
            registry.acknowledge(id, false);
        }
        identities.push(operation);
    }
    let later = &identities[1];
    let successors = [OperationReference {
        id: "later".into(),
        command: later.command.clone(),
        uncertain: false,
        fingerprint: Some(later.fingerprint),
        target: Some(later.target.unwrap().1),
    }];
    // When / Then
    let mut restarted = OperationRegistry::<()>::new("new".into());
    for user_retry in [false, true] {
        let decision = restarted.admit(
            "first",
            "old",
            identities[0].clone(),
            &[],
            &RecoveryAttempt {
                sent: true,
                user_retry,
                successors: &successors,
                ..Default::default()
            },
        );
        if decision == OperationDecision::Ready {
            state.0 = true;
            effects.push("first");
        }
        assert_eq!(decision, OperationDecision::Unknown);
    }
    assert_eq!(state, (false, 3));
    assert_eq!(effects, ["first", "later", "unrelated"]);
}
