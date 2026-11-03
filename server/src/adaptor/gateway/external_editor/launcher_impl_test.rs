use super::*;

#[test]
fn test_外部編集_フォルダとファイルを一度の起動に順に渡す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().join("workflow folder");
    std::fs::create_dir(&folder).unwrap();
    let file = folder.join("definition.yml");
    std::fs::write(&file, "definition").unwrap();
    let paths = [folder, file];
    // When
    let mut launches = 0;
    open_paths_with(&paths, "Visual Studio Code", "定義", |command| {
        launches += 1;
        // Then
        let expected = open::with_command(&paths[0], "Visual Studio Code");
        assert_eq!(command.get_program(), expected.get_program());
        let mut args = expected
            .get_args()
            .map(|arg| arg.to_os_string())
            .collect::<Vec<_>>();
        args.push(paths[1].as_os_str().to_owned());
        assert_eq!(command.get_args().collect::<Vec<_>>(), args);
        #[cfg(target_os = "macos")]
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "-a",
                "Visual Studio Code",
                "--",
                paths[0].to_str().unwrap(),
                paths[1].to_str().unwrap()
            ]
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(launches, 1);
}

#[test]
fn test_外部編集_未選択なら全パスを確認して一度に渡す() {
    // Given
    let paths = [PathBuf::from("."), PathBuf::from("Cargo.toml")];
    // When
    open_paths_with(&paths, "", "定義", |command| {
        // Then
        let expected = open::commands(&paths[0]).remove(0);
        assert_eq!(command.get_program(), expected.get_program());
        let mut args = expected
            .get_args()
            .map(|arg| arg.to_os_string())
            .collect::<Vec<_>>();
        args.push(paths[1].as_os_str().to_owned());
        assert_eq!(command.get_args().collect::<Vec<_>>(), args);
        Ok(())
    })
    .unwrap();
}

#[test]
fn test_外部編集_空パスと未選択で存在しないパスは起動しない() {
    // Given / When
    for paths in [
        vec![],
        vec![PathBuf::from("."), PathBuf::from("/missing-workflow-2011")],
    ] {
        let result = open_paths_with(&paths, "", "定義", |_| panic!("起動されない"));
        // Then
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("エディタで定義を開けませんでした:"));
    }
}

#[test]
fn test_外部編集_起動失敗に対象を付けて返す() {
    // Given / When
    let error = open_paths_with(&[PathBuf::from(".")], "Code", "定義", |_| {
        Err(std::io::Error::other("launch failed"))
    })
    .unwrap_err();
    // Then
    assert_eq!(
        error.to_string(),
        "エディタで定義を開けませんでした: launch failed"
    );
}

#[test]
fn test_外部編集_エディタ指定時も存在しないパスは起動しない() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let paths = [dir.path().to_path_buf(), dir.path().join("missing.yml")];
    // When
    let result = open_paths_with(&paths, "Code", "定義", |_| panic!("起動されない"));
    // Then
    assert!(
        matches!(result, Err(EditorError::Launch(message)) if message.contains("エディタで定義を開けませんでした"))
    );
}
