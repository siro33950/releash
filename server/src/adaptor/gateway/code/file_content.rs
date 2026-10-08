//! ファイル内容参照（at_ref / at_branch_base / staged、テキスト／バイナリ）の
//! gateway 実装。git2 によるリビジョン時点のファイル内容取得を封じ込める。

use crate::adaptor::gateway::shared::git_operation;
use git2::{AttrCheckFlags, AttrValue, Blob, Repository};
use std::io::ErrorKind;
use std::path::Path;

use crate::domain::code::{CodeError, FileContentRepository, ReviewSideBytes, ReviewSideMetadata};

/// Discover a git repository from a file path.
/// Walks ancestors so deleted files whose parent directories were also removed can still resolve.
fn discover_repo(path: &Path) -> Result<Repository, git_operation::GitOperationError> {
    let mut first_error = None;
    let mut current = Some(path);
    while let Some(candidate) = current {
        match git_operation::run(|| Repository::discover(candidate)) {
            Ok(repo) => return Ok(repo),
            Err(e @ git_operation::GitOperationError::Stopped(_)) => return Err(e),
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
        current = candidate.parent();
    }
    Err(
        first_error.unwrap_or_else(|| match git_operation::run(|| Repository::discover(path)) {
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

pub fn review_blob_at_branch_base(
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
    let tree = git_operation::run(|| merge_base_commit.tree())?;
    let entry = match git_operation::run(|| tree.get_path(relative_path)) {
        Ok(entry) => entry,
        Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(None),
        Err(e) => return Err(CodeError::from(e)),
    };
    let blob = git_operation::run(|| repo.find_blob(entry.id()))?;
    Ok(Some(present(&blob)))
}

fn blob_metadata_at_ref(file_path: &str, git_ref: &str) -> Result<ReviewSideMetadata, CodeError> {
    Ok(
        match resolve_blob_at_ref(file_path, git_ref, |blob| blob.size() as u64)? {
            Some(size_bytes) => ReviewSideMetadata::Present { size_bytes },
            None => ReviewSideMetadata::Missing,
        },
    )
}

pub fn review_blob_at_ref(file_path: &str, git_ref: &str) -> Result<ReviewSideBytes, CodeError> {
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

    let obj = match git_operation::run(|| repo.revparse_single(git_ref)) {
        Ok(obj) => obj,
        Err(e) if is_missing_head_ref(&e, git_ref) => return Ok(None),
        Err(e) => return Err(CodeError::from(e)),
    };
    let commit = git_operation::run(|| obj.peel_to_commit())?;
    let tree = git_operation::run(|| commit.tree())?;
    let entry = match git_operation::run(|| tree.get_path(relative_path)) {
        Ok(entry) => entry,
        Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(None),
        Err(e) => return Err(CodeError::from(e)),
    };
    let blob = git_operation::run(|| repo.find_blob(entry.id()))?;
    Ok(Some(present(&blob)))
}

fn is_missing_head_ref(error: &git_operation::GitOperationError, git_ref: &str) -> bool {
    git_ref == "HEAD"
        && matches!(
            error.code(),
            git2::ErrorCode::UnbornBranch | git2::ErrorCode::NotFound
        )
}

fn staged_blob_metadata(file_path: &str) -> Result<ReviewSideMetadata, CodeError> {
    Ok(
        match resolve_staged_blob(file_path, |blob| blob.size() as u64)? {
            Some(size_bytes) => ReviewSideMetadata::Present { size_bytes },
            None => ReviewSideMetadata::Missing,
        },
    )
}

pub fn review_blob_staged(file_path: &str) -> Result<ReviewSideBytes, CodeError> {
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

    let index = git_operation::run(|| repo.index())?;

    let relative_str = relative_path
        .to_str()
        .ok_or_else(|| CodeError::Rule("invalid path encoding".to_string()))?;

    let Some(entry) = git_operation::run(|| Ok(index.get_path(Path::new(relative_str), 0)))? else {
        return Ok(None);
    };

    let blob = git_operation::run(|| repo.find_blob(entry.id))?;
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

pub fn review_working_tree_bytes(file_path: &str) -> Result<ReviewSideBytes, CodeError> {
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

pub fn binary_by_attributes(file_path: &str) -> Result<bool, CodeError> {
    let path = Path::new(file_path);
    let repo = discover_repo(path)?;
    let relative_path = open_relative(path, &repo)?;
    let flags = AttrCheckFlags::FILE_THEN_INDEX;
    let binary = AttrValue::from_string(git_operation::run(|| {
        repo.get_attr(relative_path, "binary", flags)
    })?);
    if matches!(binary, AttrValue::True) {
        return Ok(true);
    }
    let diff = AttrValue::from_string(git_operation::run(|| {
        repo.get_attr(relative_path, "diff", flags)
    })?);
    Ok(matches!(
        diff,
        AttrValue::False | AttrValue::String("binary")
    ))
}

/// `FileContentRepository` の git2 実装。
pub struct FileContentGateway;

impl FileContentRepository for FileContentGateway {
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
