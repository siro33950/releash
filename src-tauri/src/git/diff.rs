use super::error::GitError;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use git2::Repository;
use std::path::Path;

/// merge-base コミットのファイル内容を取得する（テキスト）
pub fn get_file_at_branch_base(file_path: String) -> Result<String, GitError> {
    let path = Path::new(&file_path);
    let repo = Repository::discover(path)?;

    let repo_workdir = repo
        .workdir()
        .ok_or_else(|| GitError::Custom("bare repository".to_string()))?;

    let relative_path = path.strip_prefix(repo_workdir)?;

    let merge_base_commit = find_merge_base_commit(&repo)?;
    let tree = merge_base_commit.tree()?;
    let entry = tree.get_path(relative_path)?;
    let blob = repo.find_blob(entry.id())?;

    let content = std::str::from_utf8(blob.content())?.to_string();
    Ok(content)
}

/// merge-base コミットのファイル内容を取得する（バイナリ → Base64）
pub fn get_binary_file_at_branch_base(file_path: String) -> Result<String, GitError> {
    let path = Path::new(&file_path);
    let repo = Repository::discover(path)?;

    let repo_workdir = repo
        .workdir()
        .ok_or_else(|| GitError::Custom("bare repository".to_string()))?;

    let relative_path = path.strip_prefix(repo_workdir)?;

    let merge_base_commit = find_merge_base_commit(&repo)?;
    let tree = merge_base_commit.tree()?;
    let entry = tree.get_path(relative_path)?;
    let blob = repo.find_blob(entry.id())?;

    Ok(STANDARD.encode(blob.content()))
}

/// 現在のブランチと設定された base ブランチの merge-base コミットを返す。
/// Detached HEAD の場合は HEAD コミットにフォールバック。
fn find_merge_base_commit(repo: &Repository) -> Result<git2::Commit<'_>, GitError> {
    let head = repo.head().map_err(|e| {
        if e.code() == git2::ErrorCode::UnbornBranch {
            GitError::Custom("unborn branch: no commits yet".to_string())
        } else {
            GitError::from(e)
        }
    })?;

    let current_oid = head
        .target()
        .ok_or_else(|| GitError::Custom("HEAD has no target".to_string()))?;

    // Detached HEAD → HEAD コミットにフォールバック
    if !head.is_branch() {
        return Ok(repo.find_commit(current_oid)?);
    }

    let branch_name = head
        .shorthand()
        .ok_or_else(|| GitError::Custom("HEAD has no shorthand".to_string()))?;

    let config = repo.config().ok();
    let base_branch_name =
        match super::config::resolve_branch_base(repo, config.as_ref(), branch_name) {
            Some(name) => name,
            None => return Ok(repo.find_commit(current_oid)?),
        };

    // base ブランチの OID を取得
    let base_ref = format!("refs/heads/{base_branch_name}");
    let base_oid = match repo.revparse_single(&base_ref) {
        Ok(obj) => obj.peel_to_commit().map(|c| c.id()),
        Err(_) => {
            // ローカルにない場合 origin/<name> を試す
            let remote_ref = format!("refs/remotes/origin/{base_branch_name}");
            repo.revparse_single(&remote_ref)
                .map_err(|_| {
                    GitError::Custom(format!("base branch '{base_branch_name}' not found"))
                })?
                .peel_to_commit()
                .map(|c| c.id())
        }
    }
    .map_err(|e| GitError::Custom(format!("failed to resolve base branch: {e}")))?;

    let merge_base_oid = repo.merge_base(current_oid, base_oid)?;
    Ok(repo.find_commit(merge_base_oid)?)
}

pub fn get_file_at_ref(file_path: String, git_ref: String) -> Result<String, GitError> {
    let path = Path::new(&file_path);
    let repo = Repository::discover(path)?;

    let repo_workdir = repo
        .workdir()
        .ok_or_else(|| GitError::Custom("bare repository".to_string()))?;

    let relative_path = path.strip_prefix(repo_workdir)?;

    let obj = repo.revparse_single(&git_ref)?;
    let commit = obj.peel_to_commit()?;
    let tree = commit.tree()?;
    let entry = tree.get_path(relative_path)?;
    let blob = repo.find_blob(entry.id())?;

    let content = std::str::from_utf8(blob.content())?.to_string();
    Ok(content)
}

pub fn get_staged_content(file_path: String) -> Result<String, GitError> {
    let path = Path::new(&file_path);
    let repo = Repository::discover(path)?;

    let repo_workdir = repo
        .workdir()
        .ok_or_else(|| GitError::Custom("bare repository".to_string()))?;

    let relative_path = path.strip_prefix(repo_workdir)?;

    let index = repo.index()?;

    let relative_str = relative_path
        .to_str()
        .ok_or_else(|| GitError::Custom("invalid path encoding".to_string()))?;

    let entry = index
        .get_path(Path::new(relative_str), 0)
        .ok_or_else(|| GitError::Custom("file not staged".to_string()))?;

    let blob = repo.find_blob(entry.id)?;

    let content = std::str::from_utf8(blob.content())?.to_string();
    Ok(content)
}

pub fn get_binary_staged_content(file_path: String) -> Result<String, GitError> {
    let path = Path::new(&file_path);
    let repo = Repository::discover(path)?;

    let repo_workdir = repo
        .workdir()
        .ok_or_else(|| GitError::Custom("bare repository".to_string()))?;

    let relative_path = path.strip_prefix(repo_workdir)?;

    let index = repo.index()?;

    let relative_str = relative_path
        .to_str()
        .ok_or_else(|| GitError::Custom("invalid path encoding".to_string()))?;

    let entry = index
        .get_path(Path::new(relative_str), 0)
        .ok_or_else(|| GitError::Custom("file not staged".to_string()))?;

    let blob = repo.find_blob(entry.id)?;

    Ok(STANDARD.encode(blob.content()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_helpers::*;
    use git2::build::CheckoutBuilder;

    /// macOS では TempDir::path() が /var/... を返すが workdir() は /private/var/... を返す
    /// strip_prefix の不一致を防ぐため workdir() ベースでファイルパスを構築する
    fn workdir_file(repo: &Repository, name: &str) -> String {
        repo.workdir()
            .unwrap()
            .join(name)
            .to_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn test_get_file_at_branch_base_returns_base_content() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "hello.txt", "base content", "add hello.txt");

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &head, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
        add_and_commit(&repo, "hello.txt", "modified content", "modify hello.txt");

        let result = get_file_at_branch_base(workdir_file(&repo, "hello.txt")).unwrap();
        assert_eq!(result, "base content");
    }

    #[test]
    fn test_get_file_at_branch_base_detached_head() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let oid = add_and_commit(&repo, "hello.txt", "content", "add");
        repo.set_head_detached(oid).unwrap();

        let result = get_file_at_branch_base(workdir_file(&repo, "hello.txt")).unwrap();
        assert_eq!(result, "content");
    }

    #[test]
    fn test_get_file_at_branch_base_unborn_branch() {
        let (_dir, repo) = create_test_repo();
        let file_path = workdir_file(&repo, "hello.txt");
        std::fs::write(&file_path, "new file").unwrap();
        let result = get_file_at_branch_base(file_path);
        assert!(result.is_err(), "expected error, got: {:?}", result);
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("unborn branch"),
            "expected 'unborn branch' in error message, got: {err_msg}"
        );
    }

    #[test]
    fn test_get_binary_file_at_branch_base() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "img.bin", "binary data", "add binary");

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &head, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
        add_and_commit(&repo, "img.bin", "new binary", "modify binary");

        let result = get_binary_file_at_branch_base(workdir_file(&repo, "img.bin")).unwrap();
        assert_eq!(result, STANDARD.encode(b"binary data"));
    }
}
