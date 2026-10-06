pub fn backend_executable() -> std::path::PathBuf {
    let executable = std::env::current_exe().expect("test executable");
    backend_path(&executable)
}

pub(super) fn backend_path(executable: &std::path::Path) -> std::path::PathBuf {
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
