use super::*;

#[test]
fn test_外部エディタ_既定と指定の起動先を既存どおり渡す() {
    // Given
    let file = tempfile::NamedTempFile::new().unwrap();
    let path = file.path().to_str().unwrap();
    // When / Then
    for (editor, expected) in [
        ("", None),
        ("Visual Studio Code", Some("Visual Studio Code")),
    ] {
        let mut calls = 0;
        open_path_with(path, editor, "ファイル", |actual, selected| {
            calls += 1;
            assert_eq!(actual, path);
            assert_eq!(selected, expected);
            Ok(())
        })
        .unwrap();
        assert_eq!(calls, 1);
    }
}

#[test]
fn test_外部エディタ_不存在と起動失敗を握りつぶさない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing");
    // When / Then
    let error = open_path_with(missing.to_str().unwrap(), "", "ファイル", |_, _| {
        panic!("must not launch missing default path")
    })
    .unwrap_err();
    assert!(error.starts_with("ファイルを開けませんでした:"));
    let error = open_path_with(
        "/literal/$(command)",
        "editor",
        "ファイル",
        |path, _| {
            assert_eq!(path, "/literal/$(command)");
            Err(std::io::Error::other("launch failed"))
        },
    )
    .unwrap_err();
    assert_eq!(error, "エディタでファイルを開けませんでした: launch failed");
}
