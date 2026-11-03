use super::*;

fn facet(source: WorkflowSource, content: &str) -> Facet {
    Facet::restore(
        FacetKind::Policy,
        FacetKey::new("coding").unwrap(),
        "/workflows".into(),
        source,
        content.into(),
    )
}

fn file_facet(content: &str) -> Facet {
    facet(
        WorkflowSource::File {
            path: "/actual/coding.md".into(),
        },
        content,
    )
}

fn validation<T: std::fmt::Debug>(result: Result<T, WorkflowError>, reason: &str) {
    assert_eq!(result.unwrap_err(), WorkflowError::validation(reason));
}

#[test]
fn test_組み込みファセット_変更と外部編集を理由付きで拒否する() {
    // Given
    let mut facet = facet(WorkflowSource::Builtin, "original");
    // When / Then
    validation(
        facet.update_content("changed", false, &[]),
        "ビルトインファセットは編集できません",
    );
    validation(
        facet.ensure_deletable(),
        "ビルトインファセットは削除できません",
    );
    validation(
        facet.file_path(),
        "ビルトインファセットは外部エディタで開けません",
    );
    validation(
        facet.editor_paths(),
        "ビルトインファセットは外部エディタで開けません",
    );
    assert_eq!(facet.content(), "original");
}

#[test]
fn test_利用者ファセット_組み込みと同名でも更新と削除と外部編集ができる() {
    // Given
    let mut facet = file_facet("original");
    // When
    facet
        .update_content("changed {{ input.value }}", false, &["coding".into()])
        .unwrap();
    // Then
    assert_eq!(facet.content(), "changed {{ input.value }}");
    assert_eq!(facet.kind(), FacetKind::Policy);
    assert_eq!(facet.key().as_str(), "coding");
    assert_eq!(
        facet.source(),
        &WorkflowSource::File {
            path: "/actual/coding.md".into()
        }
    );
    assert_eq!(facet.ensure_deletable(), Ok(()));
    assert_eq!(facet.file_path().unwrap(), Path::new("/actual/coding.md"));
    assert_eq!(
        facet.editor_paths().unwrap(),
        [
            PathBuf::from("/workflows"),
            PathBuf::from("/actual/coding.md")
        ]
    );
}

#[test]
fn test_本文更新_未定義変数と新規の重複を拒否し本文を保つ() {
    // Given
    let mut facet = file_facet("original");
    // When / Then
    validation(
        facet.update_content("{{ bad/name }} {{ input.field }} {{ a b }}", false, &[]),
        "未定義のテンプレート変数が含まれています: bad/name, a b",
    );
    assert_eq!(facet.content(), "original");
    validation(
        facet.update_content("changed", true, &["coding".into()]),
        "ファセット 'coding' は既に存在します",
    );
    assert_eq!(facet.content(), "original");
    facet.update_content("new", true, &[]).unwrap();
    assert_eq!(facet.content(), "new");
}

#[test]
fn test_ファセット複製_識別子と重複を検証し組み込みから利用者ファイルを作る() {
    // Given
    let original = facet(WorkflowSource::Builtin, "body");
    // When / Then
    assert!(matches!(
        original.duplicate("../bad", &[]),
        Err(WorkflowError::Validation(_))
    ));
    validation(
        original.duplicate("copy", &["copy".into()]),
        "ファセット 'copy' は既に存在します",
    );
    let duplicate = original.duplicate("copy", &[]).unwrap();
    assert_eq!(duplicate.kind(), original.kind());
    assert_eq!(duplicate.key().as_str(), "copy");
    assert_eq!(duplicate.content(), "body");
    assert_eq!(
        duplicate.editor_paths().unwrap(),
        [
            PathBuf::from("/workflows"),
            PathBuf::from("/workflows/policies/copy.md")
        ]
    );
    assert_eq!(original.content(), "body");
}
