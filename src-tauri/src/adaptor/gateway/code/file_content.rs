//! ファイル内容参照（at_ref / at_branch_base / staged、テキスト／バイナリ）の
//! gateway 実装。git2 によるリビジョン時点のファイル内容取得を封じ込める。

use base64::{engine::general_purpose::STANDARD, Engine as _};
use git2::{AttrCheckFlags, AttrValue, Blob, Repository};
use std::io::ErrorKind;
use std::path::Path;

use crate::domain::code::{CodeError, FileContentRepository, ReviewSideBytes, ReviewSideMetadata};

/// Discover a git repository from a file path.
/// Walks ancestors so deleted files whose parent directories were also removed can still resolve.
fn discover_repo(path: &Path) -> Result<Repository, git2::Error> {
    let mut first_error = None;
    let mut current = Some(path);
    while let Some(candidate) = current {
        match Repository::discover(candidate) {
            Ok(repo) => return Ok(repo),
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
        current = candidate.parent();
    }
    Err(
        first_error.unwrap_or_else(|| match Repository::discover(path) {
            Ok(_) => unreachable!("repository discovery should fail consistently"),
            Err(error) => error,
        }),
    )
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

fn blob_metadata_at_branch_base(
    file_path: &str,
    base_commit_oid: Option<&str>,
) -> Result<ReviewSideMetadata, CodeError> {
    Ok(
        match resolve_blob_at_branch_base(file_path, base_commit_oid, |blob| blob.size() as u64)? {
            Some(size_bytes) => ReviewSideMetadata::Present { size_bytes },
            None => ReviewSideMetadata::Missing,
        },
    )
}

fn review_blob_at_branch_base(
    file_path: &str,
    base_commit_oid: Option<&str>,
) -> Result<ReviewSideBytes, CodeError> {
    Ok(
        match resolve_blob_at_branch_base(file_path, base_commit_oid, |blob| {
            blob.content().to_vec()
        })? {
            Some(bytes) => ReviewSideBytes::Present(bytes),
            None => ReviewSideBytes::Missing,
        },
    )
}

fn resolve_blob_at_branch_base<T>(
    file_path: &str,
    base_commit_oid: Option<&str>,
    present: impl FnOnce(&Blob<'_>) -> T,
) -> Result<Option<T>, CodeError> {
    let path = Path::new(file_path);
    let repo = discover_repo(path)?;
    let relative_path = open_relative(path, &repo)?;

    let merge_base_commit = super::resolve_merge_base_commit(&repo, base_commit_oid)?;
    let tree = merge_base_commit.tree()?;
    let entry = match tree.get_path(relative_path) {
        Ok(entry) => entry,
        Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(None),
        Err(e) => return Err(CodeError::from(e)),
    };
    let blob = repo.find_blob(entry.id())?;
    Ok(Some(present(&blob)))
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

fn blob_metadata_at_ref(file_path: &str, git_ref: &str) -> Result<ReviewSideMetadata, CodeError> {
    Ok(
        match resolve_blob_at_ref(file_path, git_ref, |blob| blob.size() as u64)? {
            Some(size_bytes) => ReviewSideMetadata::Present { size_bytes },
            None => ReviewSideMetadata::Missing,
        },
    )
}

fn review_blob_at_ref(file_path: &str, git_ref: &str) -> Result<ReviewSideBytes, CodeError> {
    Ok(
        match resolve_blob_at_ref(file_path, git_ref, |blob| blob.content().to_vec())? {
            Some(bytes) => ReviewSideBytes::Present(bytes),
            None => ReviewSideBytes::Missing,
        },
    )
}

fn resolve_blob_at_ref<T>(
    file_path: &str,
    git_ref: &str,
    present: impl FnOnce(&Blob<'_>) -> T,
) -> Result<Option<T>, CodeError> {
    let path = Path::new(file_path);
    let repo = discover_repo(path)?;
    let relative_path = open_relative(path, &repo)?;

    let obj = match repo.revparse_single(git_ref) {
        Ok(obj) => obj,
        Err(e) if is_missing_head_ref(&e, git_ref) => return Ok(None),
        Err(e) => return Err(CodeError::from(e)),
    };
    let commit = obj.peel_to_commit()?;
    let tree = commit.tree()?;
    let entry = match tree.get_path(relative_path) {
        Ok(entry) => entry,
        Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(None),
        Err(e) => return Err(CodeError::from(e)),
    };
    let blob = repo.find_blob(entry.id())?;
    Ok(Some(present(&blob)))
}

fn is_missing_head_ref(error: &git2::Error, git_ref: &str) -> bool {
    git_ref == "HEAD"
        && matches!(
            error.code(),
            git2::ErrorCode::UnbornBranch | git2::ErrorCode::NotFound
        )
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

fn staged_blob_metadata(file_path: &str) -> Result<ReviewSideMetadata, CodeError> {
    Ok(
        match resolve_staged_blob(file_path, |blob| blob.size() as u64)? {
            Some(size_bytes) => ReviewSideMetadata::Present { size_bytes },
            None => ReviewSideMetadata::Missing,
        },
    )
}

fn review_blob_staged(file_path: &str) -> Result<ReviewSideBytes, CodeError> {
    Ok(
        match resolve_staged_blob(file_path, |blob| blob.content().to_vec())? {
            Some(bytes) => ReviewSideBytes::Present(bytes),
            None => ReviewSideBytes::Missing,
        },
    )
}

fn resolve_staged_blob<T>(
    file_path: &str,
    present: impl FnOnce(&Blob<'_>) -> T,
) -> Result<Option<T>, CodeError> {
    let path = Path::new(file_path);
    let repo = discover_repo(path)?;
    let relative_path = open_relative(path, &repo)?;

    let index = repo.index()?;

    let relative_str = relative_path
        .to_str()
        .ok_or_else(|| CodeError::Rule("invalid path encoding".to_string()))?;

    let Some(entry) = index.get_path(Path::new(relative_str), 0) else {
        return Ok(None);
    };

    let blob = repo.find_blob(entry.id)?;
    Ok(Some(present(&blob)))
}

fn working_tree_metadata(file_path: &str) -> Result<ReviewSideMetadata, CodeError> {
    match regular_working_tree_metadata(file_path)? {
        Some(metadata) => Ok(ReviewSideMetadata::Present {
            size_bytes: metadata.len(),
        }),
        None => Ok(ReviewSideMetadata::Missing),
    }
}

fn review_working_tree_bytes(file_path: &str) -> Result<ReviewSideBytes, CodeError> {
    if regular_working_tree_metadata(file_path)?.is_none() {
        return Ok(ReviewSideBytes::Missing);
    }
    match std::fs::read(file_path) {
        Ok(bytes) => Ok(ReviewSideBytes::Present(bytes)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(ReviewSideBytes::Missing),
        Err(e) => Err(CodeError::from(e)),
    }
}

fn regular_working_tree_metadata(file_path: &str) -> Result<Option<std::fs::Metadata>, CodeError> {
    match std::fs::symlink_metadata(file_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(None),
        Ok(metadata) => Ok(Some(metadata)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CodeError::from(e)),
    }
}

fn binary_by_attributes(file_path: &str) -> Result<bool, CodeError> {
    let path = Path::new(file_path);
    let repo = discover_repo(path)?;
    let relative_path = open_relative(path, &repo)?;
    let flags = AttrCheckFlags::FILE_THEN_INDEX;
    let binary = AttrValue::from_string(repo.get_attr(relative_path, "binary", flags)?);
    if matches!(binary, AttrValue::True) {
        return Ok(true);
    }
    let diff = AttrValue::from_string(repo.get_attr(relative_path, "diff", flags)?);
    Ok(matches!(
        diff,
        AttrValue::False | AttrValue::String("binary")
    ))
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
    fn review_file_metadata_at_ref(
        &self,
        file_path: &str,
        git_ref: &str,
    ) -> Result<ReviewSideMetadata, CodeError> {
        blob_metadata_at_ref(file_path, git_ref)
    }
    fn review_file_bytes_at_ref(
        &self,
        file_path: &str,
        git_ref: &str,
    ) -> Result<ReviewSideBytes, CodeError> {
        review_blob_at_ref(file_path, git_ref)
    }
    fn review_file_metadata_at_branch_base(
        &self,
        file_path: &str,
        base_commit_oid: Option<&str>,
    ) -> Result<ReviewSideMetadata, CodeError> {
        blob_metadata_at_branch_base(file_path, base_commit_oid)
    }
    fn review_file_bytes_at_branch_base(
        &self,
        file_path: &str,
        base_commit_oid: Option<&str>,
    ) -> Result<ReviewSideBytes, CodeError> {
        review_blob_at_branch_base(file_path, base_commit_oid)
    }
    fn review_staged_metadata(&self, file_path: &str) -> Result<ReviewSideMetadata, CodeError> {
        staged_blob_metadata(file_path)
    }
    fn review_staged_bytes(&self, file_path: &str) -> Result<ReviewSideBytes, CodeError> {
        review_blob_staged(file_path)
    }
    fn review_working_tree_metadata(
        &self,
        file_path: &str,
    ) -> Result<ReviewSideMetadata, CodeError> {
        working_tree_metadata(file_path)
    }
    fn review_working_tree_bytes(&self, file_path: &str) -> Result<ReviewSideBytes, CodeError> {
        review_working_tree_bytes(file_path)
    }
    fn review_binary_by_attributes(&self, file_path: &str) -> Result<bool, CodeError> {
        binary_by_attributes(file_path)
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

    #[test]
    fn review_metadata_at_ref_missing_blob_returns_missing() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "tracked.txt", "content\n", "add tracked");

        let gateway = FileContentGateway;
        let result = gateway
            .review_file_metadata_at_ref(&workdir_file(&repo, "missing.txt"), "HEAD")
            .unwrap();

        assert_eq!(result, ReviewSideMetadata::Missing);
    }

    #[test]
    fn review_at_head_on_unborn_branch_returns_missing() {
        let (_dir, repo) = create_test_repo();
        let file_path = workdir_file(&repo, "new.txt");
        std::fs::write(&file_path, "new\n").unwrap();

        let gateway = FileContentGateway;
        let metadata = gateway
            .review_file_metadata_at_ref(&file_path, "HEAD")
            .unwrap();
        let bytes = gateway
            .review_file_bytes_at_ref(&file_path, "HEAD")
            .unwrap();

        assert_eq!(metadata, ReviewSideMetadata::Missing);
        assert_eq!(bytes, ReviewSideBytes::Missing);
    }

    #[test]
    fn review_working_tree_metadata_uses_metadata_size() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let file_path = workdir_file(&repo, "large.txt");
        std::fs::write(&file_path, vec![b'x'; 1_048_577]).unwrap();

        let gateway = FileContentGateway;
        let result = gateway.review_working_tree_metadata(&file_path).unwrap();

        assert_eq!(
            result,
            ReviewSideMetadata::Present {
                size_bytes: 1_048_577
            }
        );
    }

    #[test]
    fn review_at_ref_reads_deleted_file_after_parent_directories_are_removed() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        std::fs::create_dir_all(repo.workdir().unwrap().join("src/nested")).unwrap();
        add_and_commit(
            &repo,
            "src/nested/file.txt",
            "original content\n",
            "add nested file",
        );
        let base_oid = base_commit_oid(&repo);
        std::fs::remove_dir_all(repo.workdir().unwrap().join("src")).unwrap();
        let file_path = workdir_file(&repo, "src/nested/file.txt");

        let gateway = FileContentGateway;
        let metadata = gateway
            .review_file_metadata_at_ref(&file_path, "HEAD")
            .unwrap();
        let bytes = gateway
            .review_file_bytes_at_ref(&file_path, "HEAD")
            .unwrap();
        let base_metadata = gateway
            .review_file_metadata_at_branch_base(&file_path, Some(&base_oid))
            .unwrap();
        let base_bytes = gateway
            .review_file_bytes_at_branch_base(&file_path, Some(&base_oid))
            .unwrap();

        assert_eq!(metadata, ReviewSideMetadata::Present { size_bytes: 17 });
        assert_eq!(
            bytes,
            ReviewSideBytes::Present(b"original content\n".to_vec())
        );
        assert_eq!(
            base_metadata,
            ReviewSideMetadata::Present { size_bytes: 17 }
        );
        assert_eq!(
            base_bytes,
            ReviewSideBytes::Present(b"original content\n".to_vec())
        );
    }

    #[test]
    fn review_working_tree_regular_file_still_reads_metadata_and_bytes() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let file_path = workdir_file(&repo, "regular.txt");
        std::fs::write(&file_path, "regular content").unwrap();

        let gateway = FileContentGateway;
        let metadata = gateway.review_working_tree_metadata(&file_path).unwrap();
        let bytes = gateway.review_working_tree_bytes(&file_path).unwrap();

        assert_eq!(metadata, ReviewSideMetadata::Present { size_bytes: 15 });
        assert_eq!(bytes, ReviewSideBytes::Present(b"regular content".to_vec()));
    }

    #[cfg(unix)]
    #[test]
    fn review_working_tree_symlink_does_not_return_target_metadata_or_bytes() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let outside_dir = tempfile::tempdir().unwrap();
        let outside_file = outside_dir.path().join("outside.txt");
        std::fs::write(&outside_file, "outside secret").unwrap();
        let link_path = repo.workdir().unwrap().join("linked.txt");
        std::os::unix::fs::symlink(&outside_file, &link_path).unwrap();

        let gateway = FileContentGateway;
        let metadata = gateway
            .review_working_tree_metadata(link_path.to_str().unwrap())
            .unwrap();
        let bytes = gateway
            .review_working_tree_bytes(link_path.to_str().unwrap())
            .unwrap();

        assert_eq!(metadata, ReviewSideMetadata::Missing);
        assert_eq!(bytes, ReviewSideBytes::Missing);
    }

    #[test]
    fn review_binary_by_attributes_detects_minus_diff_text_file() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        std::fs::write(workdir_file(&repo, ".gitattributes"), "*.txt -diff\n").unwrap();
        let file_path = workdir_file(&repo, "data.txt");
        std::fs::write(&file_path, "plain utf8\n").unwrap();

        let gateway = FileContentGateway;
        assert!(gateway.review_binary_by_attributes(&file_path).unwrap());
    }
}
