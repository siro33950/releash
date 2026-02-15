use std::path::PathBuf;

pub(super) fn normalize_path(path: &std::path::Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            c => components.push(c),
        }
    }
    components.iter().collect()
}

pub(super) fn validate_relative_path(path: &str, repo_root: &str) -> Result<PathBuf, String> {
    if std::path::Path::new(path).is_absolute() {
        return Err("絶対パスは拒否されます".to_string());
    }
    let root = std::path::Path::new(repo_root)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let resolved = normalize_path(&root.join(path));
    if !resolved.starts_with(&root) {
        return Err("プロジェクトルート外のパスは拒否されます".to_string());
    }
    if resolved.exists() {
        let canonical = resolved.canonicalize().map_err(|e| e.to_string())?;
        if !canonical.starts_with(&root) {
            return Err(
                "シンボリックリンクによるプロジェクトルート外へのアクセスは拒否されます"
                    .to_string(),
            );
        }
    }
    Ok(resolved)
}

pub(super) fn validate_patch_paths(patch: &str, repo_root: &str) -> Result<(), String> {
    for line in patch.lines() {
        let path = line
            .strip_prefix("--- a/")
            .or_else(|| line.strip_prefix("+++ b/"));
        if let Some(p) = path {
            if p != "/dev/null" {
                validate_relative_path(p, repo_root)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_relative_path_rejects_absolute() {
        #[cfg(unix)]
        {
            let result = validate_relative_path("/etc/passwd", "/tmp");
            assert!(result.is_err());
        }
        #[cfg(windows)]
        {
            let dir = tempfile::TempDir::new().unwrap();
            let result = validate_relative_path(
                "C:\\Windows\\System32\\cmd.exe",
                dir.path().to_str().unwrap(),
            );
            assert!(result.is_err());
        }
    }

    #[test]
    fn test_validate_relative_path_rejects_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = validate_relative_path("../../etc/passwd", dir.path().to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_relative_path_rejects_nested_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("foo")).unwrap();
        let result = validate_relative_path("foo/../../etc/passwd", dir.path().to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_relative_path_accepts_valid_subdir() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        let result = validate_relative_path("src/main.rs", dir.path().to_str().unwrap());
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_patch_paths_rejects_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        let patch = "--- a/../../etc/passwd\n+++ b/../../etc/shadow\n";
        let result = validate_patch_paths(patch, dir.path().to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_patch_paths_accepts_valid_paths() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "").unwrap();
        let patch = "--- a/src/lib.rs\n+++ b/src/lib.rs\n";
        let result = validate_patch_paths(patch, dir.path().to_str().unwrap());
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_patch_paths_allows_dev_null() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("new_file.rs"), "").unwrap();
        let patch = "--- /dev/null\n+++ b/new_file.rs\n";
        let result = validate_patch_paths(patch, dir.path().to_str().unwrap());
        assert!(result.is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn test_validate_relative_path_rejects_symlink_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("inner")).unwrap();
        std::os::unix::fs::symlink("/etc", dir.path().join("inner/secret_link")).unwrap();
        let result =
            validate_relative_path("inner/secret_link/passwd", dir.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("シンボリックリンク"));
    }

    #[test]
    fn test_normalize_path_removes_parent_dir() {
        let path = std::path::Path::new("/home/user/../etc/passwd");
        let normalized = normalize_path(path);
        assert_eq!(normalized, std::path::PathBuf::from("/home/etc/passwd"));
    }

    #[test]
    fn test_normalize_path_removes_cur_dir() {
        let path = std::path::Path::new("/home/./user/./file.txt");
        let normalized = normalize_path(path);
        assert_eq!(normalized, std::path::PathBuf::from("/home/user/file.txt"));
    }
}
