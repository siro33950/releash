use super::*;
#[test]
fn test_pane配置_復元するタブと分割を検証する() {
    // Given
    let pane = PaneLayout::Pane {
        id: "pane".into(),
        tabs: vec![PaneTab {
            id: "tab".into(),
            kind: PaneTabKind::File,
        }],
        active_tab: Some("tab".into()),
    };
    // When / Then
    assert!(pane.validate().is_ok());
    let bad = PaneLayout::Pane {
        id: "pane".into(),
        tabs: vec![],
        active_tab: Some("missing".into()),
    };
    assert!(bad.validate().is_err());
    for ratio in [0.0, 1.0, f64::NAN, 0.5] {
        let split = PaneLayout::Split {
            id: "split".into(),
            axis: SplitAxis::Horizontal,
            ratio,
            first: Box::new(pane.clone()),
            second: Box::new(pane.clone()),
        };
        assert!(split.validate().is_err());
    }
}

#[test]
fn test_pane配置_異なるidの分割比率と全種類のタブを復元できる() {
    // Given
    let layout = PaneLayout::Split {
        id: "split".into(),
        axis: SplitAxis::Vertical,
        ratio: 0.7,
        first: Box::new(PaneLayout::Pane {
            id: "one".into(),
            tabs: vec![PaneTab {
                id: "terminal".into(),
                kind: PaneTabKind::Terminal,
            }],
            active_tab: Some("terminal".into()),
        }),
        second: Box::new(PaneLayout::Pane {
            id: "two".into(),
            tabs: vec![
                PaneTab {
                    id: "workflow".into(),
                    kind: PaneTabKind::Workflow,
                },
                PaneTab {
                    id: "file".into(),
                    kind: PaneTabKind::File,
                },
            ],
            active_tab: Some("file".into()),
        }),
    };
    // When / Then
    assert!(layout.validate().is_ok());
}
