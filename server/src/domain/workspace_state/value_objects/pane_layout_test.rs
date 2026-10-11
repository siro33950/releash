use super::*;

fn tab(id: &str, kind: PaneTabKind) -> PaneTab {
    PaneTab {
        id: id.into(),
        kind,
    }
}
fn pane(id: &str) -> PaneLayout {
    PaneLayout::pane(id.into(), vec![], None).unwrap()
}

#[test]
fn test_pane配置_復元するタブと分割を検証する() {
    // Given
    let valid = PaneLayout::pane(
        "pane".into(),
        vec![tab("tab", PaneTabKind::File)],
        Some("tab".into()),
    )
    .unwrap();
    // When / Then
    assert!(PaneLayout::pane("pane".into(), vec![], Some("missing".into())).is_err());
    for ratio in [0.0, 1.0, -0.1, f64::NAN, f64::INFINITY, 0.5] {
        assert_eq!(
            PaneLayout::split(
                "split".into(),
                SplitAxis::Horizontal,
                ratio,
                valid.clone(),
                pane("other")
            )
            .is_ok(),
            ratio == 0.5
        );
    }
}

#[test]
fn test_pane配置_異なるidの分割比率と全種類のタブを復元できる() {
    // Given
    let first = PaneLayout::pane(
        "one".into(),
        vec![tab("terminal", PaneTabKind::Terminal)],
        Some("terminal".into()),
    )
    .unwrap();
    let second = PaneLayout::pane(
        "two".into(),
        vec![
            tab("workflow", PaneTabKind::Workflow),
            tab("file", PaneTabKind::File),
            tab("session", PaneTabKind::Session),
        ],
        Some("file".into()),
    )
    .unwrap();
    // When
    let layout =
        PaneLayout::split("split".into(), SplitAxis::Vertical, 0.7, first, second).unwrap();
    // Then
    assert!(matches!(
        layout.view(),
        PaneLayoutView::Split {
            axis: SplitAxis::Vertical,
            ratio: 0.7,
            ..
        }
    ));
}

#[test]
fn test_pane配置_空白のpaneとsplitとtabのidを拒む() {
    // Given / When / Then
    for id in ["", " "] {
        assert!(PaneLayout::pane(id.into(), vec![], None).is_err());
        assert!(PaneLayout::pane("pane".into(), vec![tab(id, PaneTabKind::File)], None).is_err());
        assert!(
            PaneLayout::split(id.into(), SplitAxis::Horizontal, 0.5, pane("a"), pane("b")).is_err()
        );
    }
}

#[test]
fn test_pane配置_全paneとsplitとtabを通してidの重複を拒む() {
    // Given / When / Then
    assert!(PaneLayout::pane(
        "pane".into(),
        vec![
            tab("same", PaneTabKind::File),
            tab("same", PaneTabKind::Terminal)
        ],
        None
    )
    .is_err());
    assert!(PaneLayout::pane("same".into(), vec![tab("same", PaneTabKind::File)], None).is_err());
    let first = PaneLayout::pane("a".into(), vec![tab("tab", PaneTabKind::File)], None).unwrap();
    let second = PaneLayout::pane("b".into(), vec![tab("tab", PaneTabKind::File)], None).unwrap();
    assert!(PaneLayout::split("split".into(), SplitAxis::Horizontal, 0.5, first, second).is_err());
    assert!(PaneLayout::split(
        "split".into(),
        SplitAxis::Horizontal,
        0.5,
        pane("same"),
        pane("same")
    )
    .is_err());
    assert!(
        PaneLayout::split("a".into(), SplitAxis::Horizontal, 0.5, pane("a"), pane("b")).is_err()
    );
}

#[test]
fn test_pane配置_深さ32まで受け付け33を拒む() {
    // Given
    let mut layout = pane("leaf");
    // When
    for depth in 1..=32 {
        layout = PaneLayout::split(
            format!("split{depth}"),
            SplitAxis::Horizontal,
            0.5,
            layout,
            pane(&format!("leaf{depth}")),
        )
        .unwrap();
    }
    // Then
    assert!(PaneLayout::split(
        "split33".into(),
        SplitAxis::Horizontal,
        0.5,
        layout,
        pane("leaf33")
    )
    .is_err());
}
