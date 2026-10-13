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
fn test_patch生成_改行なしと特殊なpathを保持する() {
    // Given
    let selected = hunk(1, 1, &["-old", "+new"]);
    // When
    let value = patch("file\tname", &[selected], "old", "new");
    // Then
    assert!(value.starts_with("--- \"a/file\\tname\"\n"));
    assert_eq!(value.matches("\\ No newline at end of file").count(), 2);
}
