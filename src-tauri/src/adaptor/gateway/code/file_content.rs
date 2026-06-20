//! ファイル内容参照（at_ref / at_branch_base / staged、テキスト／バイナリ）の
//! gateway 実装。git2 によるリビジョン時点のファイル内容取得を封じ込める。

use base64::{engine::general_purpose::STANDARD, Engine as _};
use git2::Repository;
use std::path::Path;

use crate::domain::code::{CodeError, FileContentRepository};

/// Discover a git repository from a file path.
/// Falls back to the parent directory if the file does not exist (e.g. deleted files).
fn discover_repo(path: &Path) -> Result<Repository, git2::Error> {
    match Repository::discover(path) {
        Ok(repo) => Ok(repo),
        Err(_) if !path.exists() => {
            if let Some(parent) = path.parent() {
                Repository::discover(parent)
            } else {
                Repository::discover(path)
            }
        }
        Err(e) => Err(e),
    }
}

fn open_relative<'a>(path: &'a Path, repo: &'a Repository) -> Result<&'a Path, CodeError> {
    let repo_workdir = repo
        .workdir()
        .ok_or_else(|| CodeError::Rule("bare repository".to_string()))?;
    Ok(path.strip_prefix(repo_workdir)?)
}

// ── blob バイト取得（branch_base / ref / staged の 3 系統） ──
// テキスト／バイナリ版で共通の blob 取得 prelude（discover → 相対化 → blob lookup）を
// 系統ごとに 1 本へ集約し、公開関数は最終 encode（UTF-8 / Base64）のみ分岐させる。
// 公開関数の戻り値・エラー種別・エラーメッセージは移行前と等価に保つ。

/// merge-base コミットの blob バイトを取得する。`base_commit_oid` は usecase が解決済みの
/// base コミット OID(hex)（`None` は detached / 未設定で HEAD フォールバック）。
fn blob_at_branch_base(
    file_path: &str,
    base_commit_oid: Option<&str>,
) -> Result<Vec<u8>, CodeError> {
    let path = Path::new(file_path);
    let repo = discover_repo(path)?;
    let relative_path = open_relative(path, &repo)?;

    let merge_base_commit = super::resolve_merge_base_commit(&repo, base_commit_oid)?;
    let tree = merge_base_commit.tree()?;
    let entry = tree.get_path(relative_path)?;
    let blob = repo.find_blob(entry.id())?;
    Ok(blob.content().to_vec())
}

/// 任意リビジョン（`git_ref`）の blob バイトを取得する。
fn blob_at_ref(file_path: &str, git_ref: &str) -> Result<Vec<u8>, CodeError> {
    let path = Path::new(file_path);
    let repo = discover_repo(path)?;
    let relative_path = open_relative(path, &repo)?;

    let obj = repo.revparse_single(git_ref)?;
    let commit = obj.peel_to_commit()?;
    let tree = commit.tree()?;
    let entry = tree.get_path(relative_path)?;
    let blob = repo.find_blob(entry.id())?;
    Ok(blob.content().to_vec())
}

/// staged（index）の blob バイトを取得する。
fn blob_staged(file_path: &str) -> Result<Vec<u8>, CodeError> {
    let path = Path::new(file_path);
    let repo = discover_repo(path)?;
    let relative_path = open_relative(path, &repo)?;

    let index = repo.index()?;

    let relative_str = relative_path
        .to_str()
        .ok_or_else(|| CodeError::Rule("invalid path encoding".to_string()))?;

    let entry = index
        .get_path(Path::new(relative_str), 0)
        .ok_or_else(|| CodeError::Rule("file not staged".to_string()))?;

    let blob = repo.find_blob(entry.id)?;
    Ok(blob.content().to_vec())
}

/// blob バイトを UTF-8 テキストへ変換する（非 UTF-8 はエラー）。
fn decode_text(bytes: &[u8]) -> Result<String, CodeError> {
    Ok(std::str::from_utf8(bytes)?.to_string())
}

/// merge-base コミットのファイル内容を取得する（テキスト）。
pub(crate) fn get_file_at_branch_base(
    file_path: &str,
    base_commit_oid: Option<&str>,
) -> Result<String, CodeError> {
    decode_text(&blob_at_branch_base(file_path, base_commit_oid)?)
}

/// merge-base コミットのファイル内容を取得する（バイナリ → Base64）。
pub(crate) fn get_binary_file_at_branch_base(
    file_path: &str,
    base_commit_oid: Option<&str>,
) -> Result<String, CodeError> {
    Ok(STANDARD.encode(blob_at_branch_base(file_path, base_commit_oid)?))
}

pub(crate) fn get_file_at_ref(file_path: &str, git_ref: &str) -> Result<String, CodeError> {
    decode_text(&blob_at_ref(file_path, git_ref)?)
}

pub(crate) fn get_binary_file_at_ref(file_path: &str, git_ref: &str) -> Result<String, CodeError> {
    Ok(STANDARD.encode(blob_at_ref(file_path, git_ref)?))
}

pub(crate) fn get_staged_content(file_path: &str) -> Result<String, CodeError> {
    decode_text(&blob_staged(file_path)?)
}

pub(crate) fn get_binary_staged_content(file_path: &str) -> Result<String, CodeError> {
    Ok(STANDARD.encode(blob_staged(file_path)?))
}

/// `FileContentRepository` の git2 実装。
pub struct FileContentGateway;

impl FileContentRepository for FileContentGateway {
    fn file_at_ref(&self, file_path: &str, git_ref: &str) -> Result<String, CodeError> {
        get_file_at_ref(file_path, git_ref)
    }
    fn binary_file_at_ref(&self, file_path: &str, git_ref: &str) -> Result<String, CodeError> {
        get_binary_file_at_ref(file_path, git_ref)
    }
    fn file_at_branch_base(
        &self,
        file_path: &str,
        base_commit_oid: Option<&str>,
    ) -> Result<String, CodeError> {
        get_file_at_branch_base(file_path, base_commit_oid)
    }
    fn binary_file_at_branch_base(
        &self,
        file_path: &str,
        base_commit_oid: Option<&str>,
    ) -> Result<String, CodeError> {
        get_binary_file_at_branch_base(file_path, base_commit_oid)
    }
    fn staged_content(&self, file_path: &str) -> Result<String, CodeError> {
        get_staged_content(file_path)
    }
    fn binary_staged_content(&self, file_path: &str) -> Result<String, CodeError> {
        get_binary_staged_content(file_path)
    }
}

#[cfg(test)]
mod file_content_gateway_tests {
    use super::*;
    use crate::git::test_helpers::*;
    use git2::build::CheckoutBuilder;

    /// macOS では TempDir::path() が /var/... を返すが workdir() は /private/var/... を返す。
    /// strip_prefix の不一致を防ぐため workdir() ベースでファイルパスを構築する。
    fn workdir_file(repo: &Repository, name: &str) -> String {
        repo.workdir()
            .unwrap()
            .join(name)
            .to_str()
            .unwrap()
            .to_string()
    }

    /// base ブランチの commit OID(hex) を返す（gateway は解決済み OID を受け取る契約）。
    fn base_commit_oid(repo: &Repository) -> String {
        repo.head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .id()
            .to_string()
    }

    #[test]
    fn test_branch_base内容_base内容を返す() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "hello.txt", "base content", "add hello.txt");

        let base_oid = base_commit_oid(&repo);
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &head, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
        add_and_commit(&repo, "hello.txt", "modified content", "modify hello.txt");

        let result =
            get_file_at_branch_base(&workdir_file(&repo, "hello.txt"), Some(&base_oid)).unwrap();
        assert_eq!(result, "base content");
    }

    #[test]
    fn test_branch_base内容_detached_head() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let oid = add_and_commit(&repo, "hello.txt", "content", "add");
        repo.set_head_detached(oid).unwrap();

        let result = get_file_at_branch_base(&workdir_file(&repo, "hello.txt"), None).unwrap();
        assert_eq!(result, "content");
    }

    #[test]
    fn test_branch_base内容_unborn_branch() {
        let (_dir, repo) = create_test_repo();
        let file_path = workdir_file(&repo, "hello.txt");
        std::fs::write(&file_path, "new file").unwrap();
        let result = get_file_at_branch_base(&file_path, None);
        assert!(result.is_err(), "expected error, got: {:?}", result);
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("unborn branch"),
            "expected 'unborn branch' in error message, got: {err_msg}"
        );
    }

    #[test]
    fn test_branch_base内容_バイナリ() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "img.bin", "binary data", "add binary");

        let base_oid = base_commit_oid(&repo);
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &head, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
        add_and_commit(&repo, "img.bin", "new binary", "modify binary");

        let result =
            get_binary_file_at_branch_base(&workdir_file(&repo, "img.bin"), Some(&base_oid))
                .unwrap();
        assert_eq!(result, STANDARD.encode(b"binary data"));
    }

    #[test]
    fn test_at_ref内容_通常テキスト() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "hello.txt", "head content\nline2\n", "add hello.txt");

        let result = get_file_at_ref(&workdir_file(&repo, "hello.txt"), "HEAD").unwrap();
        assert_eq!(result, "head content\nline2\n");
    }

    #[test]
    fn test_staged内容_通常テキスト() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "staged.txt", "committed\n", "add staged.txt");

        // ワーキングツリーを変更してステージし、staged 内容（HEAD ではなく index）を取得する。
        let file_path = workdir_file(&repo, "staged.txt");
        std::fs::write(&file_path, "staged content\nline2\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("staged.txt")).unwrap();
        index.write().unwrap();

        let result = get_staged_content(&file_path).unwrap();
        assert_eq!(result, "staged content\nline2\n");
    }

    #[test]
    fn test_at_ref内容_バイナリ() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "img.bin", "binary at HEAD", "add binary");

        let result = get_binary_file_at_ref(&workdir_file(&repo, "img.bin"), "HEAD").unwrap();
        assert_eq!(result, STANDARD.encode(b"binary at HEAD"));
    }

    #[test]
    fn test_at_ref内容_削除ファイル() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "deleted.txt", "original content\n", "add file");

        let file_path = workdir_file(&repo, "deleted.txt");
        std::fs::remove_file(&file_path).unwrap();

        let result = get_file_at_ref(&file_path, "HEAD");
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result);
        assert_eq!(result.unwrap(), "original content\n");
    }

    #[test]
    fn test_staged内容_削除ファイル() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "deleted.txt", "staged content\n", "add file");

        let file_path = workdir_file(&repo, "deleted.txt");
        std::fs::remove_file(&file_path).unwrap();

        let result = get_staged_content(&file_path);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result);
        assert_eq!(result.unwrap(), "staged content\n");
    }
}
