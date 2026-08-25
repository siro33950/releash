use super::*;
use crate::domain::terminal_surface::entities::TerminalSurface;
use crate::domain::terminal_surface::gateway::TerminalSurfaceInputUnavailableCause;
use crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem;
use crate::usecase::terminal_surface::spawn_usecase::GetOrSpawnTerminalOutcome;

#[test]
fn test_ターミナル画面_所有者変換_全所有者種別をドメイン型へ変換する() {
    let workspace = TerminalSurfaceOwner::try_from(TerminalSurfaceOwnerV1::Workspace {
        workspace_path: "/repo/".to_string(),
    })
    .unwrap();
    let session = TerminalSurfaceOwner::try_from(TerminalSurfaceOwnerV1::Session {
        workspace_path: "/repo".to_string(),
        session_id: "session-1".to_string(),
    })
    .unwrap();
    assert_eq!(workspace.workspace_identity().as_str(), "/repo");
    assert_ne!(workspace.stable_key(), session.stable_key());
}

#[test]
fn test_ターミナル画面_所有者変換_空の識別子要素を拒否する() {
    assert!(
        TerminalSurfaceOwner::try_from(TerminalSurfaceOwnerV1::Workspace {
            workspace_path: " ".to_string(),
        })
        .is_err()
    );
    assert!(
        TerminalSurfaceOwner::try_from(TerminalSurfaceOwnerV1::Session {
            workspace_path: "/repo".to_string(),
            session_id: String::new(),
        })
        .is_err()
    );
}

#[test]
fn test_ターミナル画面_所有者変換_コマンド実行_所有者を拒否する() {
    let owner = serde_json::from_value::<TerminalSurfaceOwnerV1>(serde_json::json!({
        "kind": "command",
        "workspacePath": "/repo",
        "nodeExecutionId": "node-execution-1"
    }));

    assert!(owner.is_err());
}

#[test]
fn test_ターミナル画面取得または生成_応答で接続前の復元点を二重送信しない() {
    let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap();
    let response = GetOrSpawnTerminalV1::from(GetOrSpawnTerminalOutcome {
        surface: TerminalSurface::new(1, owner, None).summary(),
        restored_from_checkpoint: false,
        is_new: true,
    });

    let json = serde_json::to_value(response).unwrap();
    assert!(json.get("terminal_surface").is_none());
    assert_eq!(json["session_key"], "workspace:5:/repo");
}

#[test]
fn test_ターミナル入力不能_全内部原因を固定文言へ変換しwireから除外する() {
    // Given
    let causes = [
        TerminalSurfaceInputUnavailableCause::StaleAttachment,
        TerminalSurfaceInputUnavailableCause::PendingCapacityExceeded,
        TerminalSurfaceInputUnavailableCause::RuntimeWriteFailed(
            "PTY writer disconnected".to_string(),
        ),
    ];

    // When / Then
    for cause in causes {
        let internal_cause = cause.internal_cause().to_string();
        let item = TerminalSurfaceStreamItemV1::from(TerminalSurfaceStreamItem::InputUnavailable {
            session_key: "terminal-1".to_string(),
            cause,
        });
        let json = serde_json::to_value(item).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "type": "input_unavailable",
                "session_key": "terminal-1",
                "message": "Terminal input could not be sent. Try again.",
            })
        );
        assert!(!json.to_string().contains(&internal_cause));
    }
}
