use super::*;

fn target(line: Option<u32>, end: Option<u32>) -> Target {
    Target {
        file_path: Some("src/main.rs".into()),
        line_number: line,
        end_line: end,
        side: None,
        end_side: None,
    }
}

/// 変更後の 1〜6 行。変更後の 4 行目の前に、変更前の 4 行目の削除がある。
fn diff() -> Vec<DiffLine> {
    let context = |old, new| DiffLine::Context {
        old,
        new,
        text: format!("line {new}"),
    };
    vec![
        context(1, 1),
        context(2, 2),
        context(3, 3),
        DiffLine::Deletion {
            old: 4,
            text: "removed".into(),
        },
        DiffLine::Addition {
            new: 4,
            text: "line 4".into(),
        },
        context(5, 5),
        context(6, 6),
    ]
}

fn numbers(excerpt: &[ExcerptLine]) -> Vec<(Option<u32>, Option<u32>, bool)> {
    excerpt
        .iter()
        .map(|line| {
            (
                line.line.old_number(),
                line.line.new_number(),
                line.targeted,
            )
        })
        .collect()
}

#[test]
fn test_対象行の切り出し_側の無い対象は変更後の行と前の3行を返す() {
    // Given
    let target = target(Some(5), None);
    // When
    let excerpt = target.excerpt(&diff());
    // Then
    assert_eq!(
        numbers(&excerpt),
        vec![
            (Some(3), Some(3), false),
            (Some(4), None, false),
            (None, Some(4), false),
            (Some(5), Some(5), true),
        ]
    );
}

#[test]
fn test_対象行の切り出し_先頭に近い対象は前の行をあるだけ返す() {
    // Given
    let target = target(Some(2), None);
    // When
    let excerpt = target.excerpt(&diff());
    // Then
    assert_eq!(
        numbers(&excerpt),
        vec![(Some(1), Some(1), false), (Some(2), Some(2), true)]
    );
}

#[test]
fn test_対象行の切り出し_範囲の対象は範囲の全行を対象の行にする() {
    // Given
    let target = target(Some(3), Some(5));
    // When
    let excerpt = target.excerpt(&diff());
    // Then
    assert_eq!(
        numbers(&excerpt),
        vec![
            (Some(1), Some(1), false),
            (Some(2), Some(2), false),
            (Some(3), Some(3), true),
            (Some(4), None, true),
            (None, Some(4), true),
            (Some(5), Some(5), true),
        ]
    );
}

#[test]
fn test_対象行の切り出し_削除の側の対象は変更前の行番号で探す() {
    // Given
    let mut target = target(Some(4), None);
    target.side = Some(LineSide::Deletions);
    // When
    let excerpt = target.excerpt(&diff());
    // Then
    assert_eq!(
        excerpt.last().map(|line| (&line.line, line.targeted)),
        Some((
            &DiffLine::Deletion {
                old: 4,
                text: "removed".into()
            },
            true
        ))
    );
}

#[test]
fn test_対象行の切り出し_変更後の行が無い差分では側の無い対象を変更前で探す() {
    // Given
    let deleted = vec![
        DiffLine::Deletion {
            old: 1,
            text: "a".into(),
        },
        DiffLine::Deletion {
            old: 2,
            text: "b".into(),
        },
    ];
    // When
    let excerpt = target(Some(2), None).excerpt(&deleted);
    // Then
    assert_eq!(
        numbers(&excerpt),
        vec![(Some(1), None, false), (Some(2), None, true)]
    );
}

#[test]
fn test_対象行の切り出し_行の無い対象と差分に無い行は空を返す() {
    // Given
    let diff = diff();
    // When / Then
    assert!(target(None, None).excerpt(&diff).is_empty());
    assert!(target(Some(7), None).excerpt(&diff).is_empty());
    assert!(target(Some(5), Some(9)).excerpt(&diff).is_empty());
}
