//! Review read model command の orchestration。
//!
//! controller と code usecase が repository state の具体 snapshot を共有しないよう、snapshot 取得、
//! version 整合、ReviewSnapshot / ReviewFileView の read model 構成をこの usecase に集約する。

use std::path::{Component, Path};
use std::sync::Arc;

use crate::domain::code::{
    CodeError, Hunk, ReviewBase, ReviewBlobSide, ReviewSideBytes, ReviewSideMetadata,
};

use crate::domain::code::value_objects::review::ReviewPresentation;

use super::code_dto::{
    DiffHunksResultDto, HunkDto, ReviewBinaryDto, ReviewFileViewDto, ReviewImageDto,
    ReviewTextDiffDto, ReviewTextSource,
};
use super::code_error::CodeUsecaseError;
use super::code_usecase::{CodeUsecase, ReviewContentSource, SelectedReviewSide};
use super::repository_dto::FileStatusDto;
use super::repository_state::snapshot::RepositorySnapshot;
use crate::domain::code::value_objects::file_list::{FileFilter, FilteredFiles};

use super::repository_state::{RepositoryStateError, RepositoryStateService};

trait ReviewSnapshotProvider: Send + Sync {
    fn snapshot(&self, worktree_path: &str) -> Result<Arc<RepositorySnapshot>, CodeUsecaseError>;
}

impl ReviewSnapshotProvider for RepositoryStateService {
    fn snapshot(&self, worktree_path: &str) -> Result<Arc<RepositorySnapshot>, CodeUsecaseError> {
        RepositoryStateService::get_snapshot(self, worktree_path).map_err(repository_state_error)
    }
}

#[async_trait::async_trait]

trait ReviewCodePort: Send + Sync {
    fn file_target(&self, worktree: &str, file: &str) -> Result<String, CodeUsecaseError>;
    fn directory_entries(
        &self,
        worktree: &str,
        directory: &str,
        statuses: &[FileStatusDto],
    ) -> Result<Vec<super::directory_query::DirectoryEntryDto>, CodeUsecaseError>;
    fn filtered_files(
        &self,
        worktree: &str,
        filter: &FileFilter,
        statuses: &[FileStatusDto],
    ) -> Result<FilteredFiles, CodeUsecaseError>;

    fn select_review_side_source(
        &self,
        file_path: &str,
        side: ReviewBlobSide,
        base: ReviewBase,
    ) -> Result<SelectedReviewSide, CodeUsecaseError>;

    fn read_review_source_bytes(
        &self,
        file_path: &str,
        source: ReviewContentSource,
    ) -> Result<ReviewSideBytes, CodeUsecaseError>;

    fn review_binary_by_attributes(&self, file_path: &str) -> Result<bool, CodeUsecaseError>;
    fn review_working_tree_matches_source(
        &self,
        file_path: &str,
        source: ReviewContentSource,
    ) -> Result<bool, CodeUsecaseError>;

    fn compute_diff_hunks(
        &self,
        original: &str,
        modified: &str,
        file_path: Option<&str>,
    ) -> Result<DiffHunksResultDto, CodeUsecaseError>;
}

#[async_trait::async_trait]

impl ReviewCodePort for CodeUsecase {
    fn file_target(&self, worktree: &str, file: &str) -> Result<String, CodeUsecaseError> {
        CodeUsecase::file_target(self, worktree, file)
    }

    fn directory_entries(
        &self,
        worktree: &str,
        directory: &str,
        statuses: &[FileStatusDto],
    ) -> Result<Vec<super::directory_query::DirectoryEntryDto>, CodeUsecaseError> {
        CodeUsecase::directory_entries(self, worktree, directory, statuses)
    }

    fn filtered_files(
        &self,
        worktree: &str,
        filter: &FileFilter,
        statuses: &[FileStatusDto],
    ) -> Result<FilteredFiles, CodeUsecaseError> {
        CodeUsecase::filtered_files(self, worktree, filter, statuses)
    }

    fn select_review_side_source(
        &self,
        file_path: &str,
        side: ReviewBlobSide,
        base: ReviewBase,
    ) -> Result<SelectedReviewSide, CodeUsecaseError> {
        CodeUsecase::select_review_side_source(self, file_path, side, base)
    }

    fn read_review_source_bytes(
        &self,
        file_path: &str,
        source: ReviewContentSource,
    ) -> Result<ReviewSideBytes, CodeUsecaseError> {
        CodeUsecase::read_review_source_bytes(self, file_path, source)
    }

    fn review_working_tree_matches_source(
        &self,
        file_path: &str,
        source: ReviewContentSource,
    ) -> Result<bool, CodeUsecaseError> {
        CodeUsecase::review_working_tree_matches_source(self, file_path, source)
    }

    fn review_binary_by_attributes(&self, file_path: &str) -> Result<bool, CodeUsecaseError> {
        CodeUsecase::review_binary_by_attributes(self, file_path)
    }

    fn compute_diff_hunks(
        &self,
        original: &str,
        modified: &str,
        file_path: Option<&str>,
    ) -> Result<DiffHunksResultDto, CodeUsecaseError> {
        CodeUsecase::compute_diff_hunks(self, original, modified, file_path)
    }
}

#[derive(Clone)]
pub struct ReviewUsecase {
    repository_state: Arc<dyn ReviewSnapshotProvider>,
    code: Arc<dyn ReviewCodePort>,
}

impl ReviewUsecase {
    pub fn new(repository_state: Arc<RepositoryStateService>, code: Arc<CodeUsecase>) -> Self {
        let repository_state: Arc<dyn ReviewSnapshotProvider> = repository_state;
        let code: Arc<dyn ReviewCodePort> = code;
        Self {
            repository_state,
            code,
        }
    }

    #[cfg(test)]
    fn new_with_ports(
        repository_state: Arc<dyn ReviewSnapshotProvider>,
        code: Arc<dyn ReviewCodePort>,
    ) -> Self {
        Self {
            repository_state,
            code,
        }
    }

    pub fn directory_entries(
        &self,
        worktree: &str,
        directory: &str,
    ) -> Result<Vec<super::directory_query::DirectoryEntryDto>, CodeUsecaseError> {
        let snapshot = self.snapshot(worktree)?;
        self.code
            .directory_entries(worktree, directory, &snapshot.status)
    }

    /// worktree の git の状態を読み、条件で絞り込んだファイル一覧を返す。
    pub fn filtered_files(
        &self,
        worktree: &str,
        filter: &FileFilter,
    ) -> Result<FilteredFiles, CodeUsecaseError> {
        let snapshot = self.snapshot(worktree)?;
        self.code.filtered_files(worktree, filter, &snapshot.status)
    }

    pub fn get_file_tab_view(
        &self,
        worktree_path: &str,
        path: &str,
        base: &str,
    ) -> Result<super::code_dto::FileTabViewDto, CodeUsecaseError> {
        use super::code_dto::FileTabViewDto;
        let base = ReviewBase::parse(base)?;
        let relative_path = resolve_review_target(worktree_path, path)?;
        let file_path = self.code.file_target(worktree_path, &relative_path)?;
        let original_source =
            self.code
                .select_review_side_source(&file_path, ReviewBlobSide::Original, base)?;
        let modified_source =
            self.code
                .select_review_side_source(&file_path, ReviewBlobSide::Modified, base)?;
        if original_source.metadata.is_missing() && modified_source.metadata.is_missing() {
            return Err(CodeError::Rule(format!("file target not found: {relative_path}")).into());
        }
        let binary_attribute =
            if crate::domain::code::value_objects::review::ReviewBlobContentType::image_from_path(
                &relative_path,
            )
            .is_none()
            {
                self.code.review_binary_by_attributes(&file_path)?
            } else {
                false
            };
        let initial_mode = ReviewPresentation::initial_file_mode(
            self.code
                .review_working_tree_matches_source(&file_path, original_source.source)?,
        );
        let (original_bytes, modified_bytes) = if binary_attribute {
            (ReviewSideBytes::Missing, ReviewSideBytes::Missing)
        } else {
            (
                self.code
                    .read_review_source_bytes(&file_path, original_source.source)?,
                self.code
                    .read_review_source_bytes(&file_path, modified_source.source)?,
            )
        };
        let mut dto = FileTabViewDto {
            view: binary_view(
                &relative_path,
                original_source.metadata,
                modified_source.metadata,
            ),
            initial_mode,
            patch: String::new(),
        };
        if ReviewPresentation::classify(&relative_path, false, &original_bytes, &modified_bytes)
            == ReviewPresentation::Image
        {
            let mime = review_blob_mime_for_path(&relative_path);
            dto.view = ReviewFileViewDto::Image(ReviewImageDto {
                path: relative_path,
                original_url: side_data_url(&original_bytes, mime),
                modified_url: side_data_url(&modified_bytes, mime),
            });
            return Ok(dto);
        }
        if ReviewPresentation::classify(
            &relative_path,
            binary_attribute,
            &original_bytes,
            &modified_bytes,
        ) == ReviewPresentation::Binary
        {
            return Ok(dto);
        }
        let original = decode_side_text(&original_bytes)?;
        let modified = decode_side_text(&modified_bytes)?;
        let hunks = self
            .code
            .compute_diff_hunks(&original, &modified, Some(&relative_path))?;
        let hunks: Vec<_> = hunks.hunks.iter().map(hunk_dto_to_domain).collect();
        dto.patch = crate::domain::code::services::file_hunk::patch(
            &relative_path,
            &hunks,
            &original,
            &modified,
        );
        dto.view = ReviewFileViewDto::TextDiff(ReviewTextDiffDto {
            path: relative_path,
            source: text_source(&original_bytes, &modified_bytes),
            original,
            modified,
        });
        Ok(dto)
    }

    fn snapshot(&self, worktree_path: &str) -> Result<Arc<RepositorySnapshot>, CodeUsecaseError> {
        self.repository_state.snapshot(worktree_path)
    }
}

fn resolve_review_target(worktree_path: &str, raw: &str) -> Result<String, CodeUsecaseError> {
    let worktree = Path::new(worktree_path);
    let path = Path::new(raw);
    let relative = if path.is_absolute() {
        path.strip_prefix(worktree)
            .map_err(|_| CodeError::Rule(format!("invalid review target path: {raw}")))?
    } else {
        path
    };

    if relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(CodeError::Rule(format!("invalid review target path: {raw}")).into());
    }

    let normalized = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            Component::CurDir => None,
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    if normalized.is_empty() {
        return Err(CodeError::Rule("empty review target path".to_string()).into());
    }
    Ok(normalized)
}

fn decode_side_text(bytes: &ReviewSideBytes) -> Result<String, CodeUsecaseError> {
    match bytes {
        ReviewSideBytes::Present(bytes) => Ok(std::str::from_utf8(bytes)
            .map_err(CodeError::from)?
            .to_string()),
        ReviewSideBytes::Missing => Ok(String::new()),
    }
}

impl ReviewSideMetadata {
    fn is_missing(self) -> bool {
        matches!(self, Self::Missing)
    }

    fn size_bytes(self) -> Option<u64> {
        match self {
            Self::Present { size_bytes } => Some(size_bytes),
            Self::Missing => None,
        }
    }
}

fn hunk_dto_to_domain(h: &HunkDto) -> Hunk {
    Hunk {
        old_start: h.old_start,
        old_lines: h.old_lines,
        new_start: h.new_start,
        new_lines: h.new_lines,
        lines: h.lines.clone(),
    }
}

fn text_source(original: &ReviewSideBytes, modified: &ReviewSideBytes) -> ReviewTextSource {
    match (original, modified) {
        (ReviewSideBytes::Missing, ReviewSideBytes::Present(_)) => ReviewTextSource::Added,
        (ReviewSideBytes::Present(_), ReviewSideBytes::Missing) => ReviewTextSource::Deleted,
        _ => ReviewTextSource::Diff,
    }
}

fn binary_view(
    relative_path: &str,
    original_metadata: ReviewSideMetadata,
    modified_metadata: ReviewSideMetadata,
) -> ReviewFileViewDto {
    ReviewFileViewDto::Binary(ReviewBinaryDto {
        path: relative_path.to_string(),
        original_size: original_metadata.size_bytes(),
        modified_size: modified_metadata.size_bytes(),
    })
}

fn review_blob_mime_for_path(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("tiff") | Some("tif") => "image/tiff",
        Some("avif") => "image/avif",
        _ => "application/octet-stream",
    }
}

fn repository_state_error(error: RepositoryStateError) -> CodeUsecaseError {
    CodeError::External(error.to_string()).into()
}

#[cfg(test)]
#[path = "review_usecase_test.rs"]
mod review_usecase_tests;

fn side_data_url(bytes: &ReviewSideBytes, mime: &str) -> Option<String> {
    use base64::Engine;
    match bytes {
        ReviewSideBytes::Present(bytes) => Some(format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )),
        ReviewSideBytes::Missing => None,
    }
}
