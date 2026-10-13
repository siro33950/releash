use super::*;

fn hunk(old: u32, new: u32, lines: &[&str]) -> Hunk {
    Hunk {
        index: 0,
        hunk_id: "hunk".into(),
        old_start: old,
        old_lines: lines.iter().filter(|l| !l.starts_with('+')).count() as u32,
        new_start: new,
        new_lines: lines.iter().filter(|l| !l.starts_with('-')).count() as u32,
        lines: lines.iter().map(|s| (*s).into()).collect(),
    }
}

#[test]
fn test_hunk操作_部分stageと離れたstageを保持する() {
    // Given
    let head = "a\nb\nc\nd\ne\nf\n";
    let index = "A\nb\nc\nd\ne\nF\n";
    let working = "A\nB\nc\nd\ne\nF\n";
    let staged = [hunk(1, 1, &["-a", "+A"]), hunk(6, 6, &["-f", "+F"])];
    let selected = hunk(1, 1, &["-a", "-b", "+A", "+B", " c"]);
    // When / Then
    assert_eq!(
        index_after_hunk(head, index, working, &staged, &selected, true).unwrap(),
        working
    );
    assert_eq!(
        index_after_hunk(head, index, working, &staged, &selected, false).unwrap(),
        "a\nb\nc\nd\ne\nF\n"
    );
}

#[test]
fn test_hunk操作_追加と最終改行なしを保持する() {
    // Given
    let selected = Hunk {
        old_lines: 0,
        ..hunk(0, 1, &["+new"])
    };
    // When / Then
    assert_eq!(
        index_after_hunk("", "", "new", &[], &selected, true).unwrap(),
        "new"
    );
}

#[test]
fn test_hunk状態_部分stageは未stageで全変更一致はstage済みになる() {
    // Given
    let selected = hunk(1, 1, &["-a", "-b", "+A", "+B"]);
    // When / Then
    let partial = [hunk(1, 1, &["-a", "+A"])];
    assert_eq!(
        hunk_stage_states(
            Some("A\nb\n"),
            "A\nB\n",
            &partial,
            std::slice::from_ref(&selected)
        ),
        vec![false]
    );
    assert_eq!(
        hunk_stage_states(
            Some("A\nB\n"),
            "A\nB\n",
            std::slice::from_ref(&selected),
            &[selected.clone()]
        ),
        vec![true]
    );
    assert_eq!(
        hunk_stage_states(Some("a\nb\n"), "A\nB\n", &[], &[selected]),
        vec![false]
    );
}

#[test]
fn test_patch生成_改行なしと特殊なpathを保持する() {
    // Given
    let selected = hunk(1, 1, &["-old", "+new"]);
    // When
    let value = patch("file\tname", &[selected], "old", "new");
    // Then
    assert!(value.starts_with("--- \"a/file\\tname\"\n"));
    assert_eq!(value.matches("\\ No newline at end of file").count(), 2);
}

#[test]
fn test_hunk操作_context行だけのstageを保持する() {
    // Given
    let staged = hunk(1, 1, &["-a", "-b", "+A", "+B", " c"]);
    let selected = hunk(1, 1, &[" a", "-b", "+B", " c"]);
    // When / Then
    assert_eq!(
        index_after_hunk(
            "a\nb\nc\n",
            "A\nB\nc\n",
            "a\nB\nc\n",
            std::slice::from_ref(&staged),
            &selected,
            true
        )
        .unwrap(),
        "A\nB\nc\n"
    );
    assert_eq!(
        index_after_hunk(
            "a\nb\nc\n",
            "A\nB\nc\n",
            "a\nB\nc\n",
            std::slice::from_ref(&staged),
            &selected,
            false
        )
        .unwrap(),
        "A\nb\nc\n"
    );
    assert_eq!(
        hunk_stage_states(Some("A\nB\nc\n"), "a\nB\nc\n", &[staged], &[selected]),
        vec![true]
    );
}

#[test]
fn test_hunk操作_同じ位置の行追加と既存行のstageを区別する() {
    // Given
    let staged = hunk(1, 1, &["+prefix", " a"]);
    let selected = hunk(1, 1, &["-a", "+A"]);
    // When / Then
    assert_eq!(
        index_after_hunk("a\n", "prefix\na\n", "A\n", &[staged], &selected, true).unwrap(),
        "prefix\nA\n"
    );
}

#[test]
fn test_ファイルhunkのstage状態_indexが非テキストならテキストhunkをstage済みとしない() {
    // Given
    let hunk = hunk(1, 1, &["-old", "+new"]);
    // When / Then
    assert_eq!(hunk_stage_states(None, "new\n", &[], &[hunk]), vec![false]);
}

#[test]
fn test_hunk受付_headのテキストの存在するhunkだけを受け付ける() {
    // Given
    let hunks = [hunk(1, 1, &["-old", "+new"])];
    // When / Then
    assert!(ensure_hunk_comparison(crate::domain::code::ReviewBase::Head).is_ok());
    assert!(ensure_hunk_comparison(crate::domain::code::ReviewBase::BranchBase).is_err());
    assert!(select_hunk(Some(&hunks), "hunk").is_ok());
    assert!(matches!(
        select_hunk(Some(&hunks), "gone"),
        Err(CodeError::StaleReviewGroupTarget { .. })
    ));
    assert!(matches!(select_hunk(None, "hunk"), Err(CodeError::Rule(_))));
}

#[test]
fn test_index用patch_追加削除変更のヘッダーと行数を保持する() {
    // Given / When / Then
    for (head, index, working, stage, before, after, lines, old_header, new_header, counts) in [
        (
            false,
            false,
            true,
            true,
            "",
            "new\n",
            vec!["+new"],
            "--- /dev/null",
            "+++ \"b/file\"",
            (1, 0),
        ),
        (
            false,
            true,
            true,
            false,
            "new\n",
            "",
            vec!["-new"],
            "--- \"a/file\"",
            "+++ /dev/null",
            (0, 1),
        ),
        (
            true,
            true,
            false,
            true,
            "old\n",
            "",
            vec!["-old"],
            "--- \"a/file\"",
            "+++ /dev/null",
            (0, 1),
        ),
        (
            true,
            false,
            false,
            false,
            "",
            "old\n",
            vec!["+old"],
            "--- /dev/null",
            "+++ \"b/file\"",
            (1, 0),
        ),
        (
            true,
            true,
            true,
            true,
            "old\n",
            "new\n",
            vec!["-old", "+new"],
            "--- \"a/file\"",
            "+++ \"b/file\"",
            (1, 1),
        ),
    ] {
        let hunks = [hunk(
            if before.is_empty() { 0 } else { 1 },
            if after.is_empty() { 0 } else { 1 },
            &lines,
        )];
        let patch = index_patch(
            "file",
            &hunks,
            before,
            after,
            FilePresence {
                head,
                index,
                working,
            },
            stage,
        );
        assert!(patch.starts_with(&format!("{old_header}\n{new_header}\n")));
        assert_eq!(line_counts(&hunks), counts);
    }
}
