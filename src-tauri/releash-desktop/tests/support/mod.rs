pub fn backend_executable() -> std::path::PathBuf {
    let executable = std::env::current_exe().expect("test executable");
    backend_path(&executable)
}

fn backend_path(executable: &std::path::Path) -> std::path::PathBuf {
    let path = executable
        .parent()
        .and_then(std::path::Path::parent)
        .expect("target profile directory")
        .join(format!("releash-backend{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.is_file(),
        "{} is missing; run cargo build -p releash-backend --bin releash-backend first",
        path.display()
    );
    path
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_サーバ実行ファイル_同じprofileの出力を使い未ビルド時に手順を示す() {
        // Given
        let directory = tempfile::tempdir().unwrap();
        let test = directory.path().join("deps/desktop-test");
        let backend = directory
            .path()
            .join(format!("releash-backend{}", std::env::consts::EXE_SUFFIX));
        // When
        let error = std::panic::catch_unwind(|| super::backend_path(&test)).unwrap_err();
        // Then
        let message = error.downcast_ref::<String>().unwrap();
        assert!(message.contains("cargo build -p releash-backend --bin releash-backend"));
        // When
        std::fs::write(&backend, []).unwrap();
        // Then
        assert_eq!(super::backend_path(&test), backend);
    }
}
