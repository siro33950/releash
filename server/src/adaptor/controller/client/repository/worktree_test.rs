use super::*;
use crate::adaptor::presenter::client::{
    create_worktrees_request::Launch, WorktreeSessionLaunch, WorktreeWorkflowLaunch,
};

#[test]
fn test_作成後の起動入力_種別と必須値とterminal寸法を検証する() {
    // Given / When / Then
    assert!(parse_launch(None).is_ok());
    let session = WorktreeSessionLaunch {
        provider: "claude".into(),
        rows: 24,
        cols: 80,
        request_id: "request".into(),
    };
    assert!(parse_launch(Some(Launch::Session(session.clone()))).is_ok());
    for invalid in [
        WorktreeSessionLaunch {
            provider: "unknown".into(),
            ..session.clone()
        },
        WorktreeSessionLaunch {
            rows: 0,
            ..session.clone()
        },
        WorktreeSessionLaunch {
            cols: 65536,
            ..session.clone()
        },
        WorktreeSessionLaunch {
            request_id: " ".into(),
            ..session
        },
    ] {
        assert!(parse_launch(Some(Launch::Session(invalid))).is_err());
    }
    assert!(parse_launch(Some(Launch::Workflow(WorktreeWorkflowLaunch {
        name: "review".into(),
        request: Some("check".into())
    })))
    .is_ok());
    assert!(parse_launch(Some(Launch::Workflow(WorktreeWorkflowLaunch {
        name: " ".into(),
        request: None
    })))
    .is_err());
}
