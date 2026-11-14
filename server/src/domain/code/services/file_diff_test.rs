use super::*;

fn hunk(old_start: u32, new_start: u32, lines: &[&str]) -> Hunk {
    Hunk {
        old_start,
        old_lines: lines.iter().filter(|l| !l.starts_with('+')).count() as u32,
        new_start,
        new_lines: lines.iter().filter(|l| !l.starts_with('-')).count() as u32,
        lines: lines.iter().map(|s| (*s).into()).collect(),
    }
}

fn context(old: u32, new: u32, text: &str) -> DiffLine {
    DiffLine::Context {
        old,
        new,
        text: text.into(),
    }
}

#[test]
fn test_ファイル全体の差分_hunkの外の行を両方にある行として並べる() {
    // Given
    let modified = "a\nb\nB\nd\ne\n";
    let hunks = [hunk(2, 2, &[" b", "-c", "+B", " d"])];
    // When
    let value = lines(&hunks, modified);
    // Then
    assert_eq!(
        value,
        vec![
            context(1, 1, "a"),
            context(2, 2, "b"),
            DiffLine::Deletion {
                old: 3,
                text: "c".into()
            },
            DiffLine::Addition {
                new: 3,
                text: "B".into()
            },
            context(4, 4, "d"),
            context(5, 5, "e"),
        ]
    );
}

#[test]
fn test_ファイル全体の差分_削除だけのhunkの前後で行番号がずれる() {
    // Given
    let modified = "a\nb\nd\n";
    let hunks = [Hunk {
        old_start: 3,
        old_lines: 1,
        new_start: 2,
        new_lines: 0,
        lines: vec!["-c".into()],
    }];
    // When
    let value = lines(&hunks, modified);
    // Then
    assert_eq!(
        value,
        vec![
            context(1, 1, "a"),
            context(2, 2, "b"),
            DiffLine::Deletion {
                old: 3,
                text: "c".into()
            },
            context(4, 3, "d"),
        ]
    );
}

#[test]
fn test_ファイル全体の差分_差分が無ければ全行が両方にある行になる() {
    // Given
    let modified = "a\r\nb";
    // When
    let value = lines(&[], modified);
    // Then
    assert_eq!(value, vec![context(1, 1, "a"), context(2, 2, "b")]);
}
