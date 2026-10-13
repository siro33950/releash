//! Review read model の表示判定に使う値オブジェクト。

use crate::domain::code::CodeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileMode {
    File,
    Diff,
}

impl FileMode {
    pub fn for_change(changed: bool) -> Self {
        if changed {
            Self::Diff
        } else {
            Self::File
        }
    }

    pub fn parse(value: &str) -> Result<Self, CodeError> {
        match value {
            "file" => Ok(Self::File),
            "diff" => Ok(Self::Diff),
            other => Err(CodeError::Rule(format!("invalid file mode: {other}"))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Diff => "diff",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReviewBase {
    Head,
    BranchBase,
}

impl ReviewBase {
    pub fn parse(value: &str) -> Result<Self, CodeError> {
        match value {
            "head" => Ok(Self::Head),
            "branch-base" => Ok(Self::BranchBase),
            other => Err(CodeError::Rule(format!("invalid review base: {other}"))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::BranchBase => "branch-base",
        }
    }

    pub fn is_branch_base(self) -> bool {
        matches!(self, Self::BranchBase)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReviewSection {
    Changes,
    Staged,
    FileTab,
}

impl ReviewSection {
    pub fn parse(value: &str) -> Result<Self, CodeError> {
        match value {
            "changes" => Ok(Self::Changes),
            "staged" => Ok(Self::Staged),
            "file-tab" => Ok(Self::FileTab),
            other => Err(CodeError::Rule(format!("invalid review section: {other}"))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Changes => "changes",
            Self::Staged => "staged",
            Self::FileTab => "file-tab",
        }
    }

    pub fn is_staged(self) -> bool {
        matches!(self, Self::Staged)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewContentSource {
    BranchBase,
    BranchBaseTip,
    Head,
    Staged,
    WorkingTree,
}

impl ReviewContentSource {
    pub fn select(
        side: super::super::ReviewBlobSide,
        section: ReviewSection,
        base: ReviewBase,
    ) -> Self {
        use super::super::ReviewBlobSide as Side;
        match (base, section, side) {
            (ReviewBase::BranchBase, ReviewSection::FileTab, Side::Original) => Self::BranchBaseTip,
            (ReviewBase::BranchBase, _, Side::Original) => Self::BranchBase,
            (ReviewBase::BranchBase, _, Side::Modified) => Self::WorkingTree,
            (_, ReviewSection::Staged, Side::Original) => Self::Head,
            (_, ReviewSection::Staged, Side::Modified) => Self::Staged,
            (_, ReviewSection::FileTab, Side::Original) => Self::Head,
            (_, ReviewSection::FileTab, Side::Modified) => Self::WorkingTree,
            (_, ReviewSection::Changes, Side::Original) => Self::Staged,
            (_, ReviewSection::Changes, Side::Modified) => Self::WorkingTree,
        }
    }
}

pub fn review_target_is_available(
    section: ReviewSection,
    base: ReviewBase,
    in_files: bool,
    in_staged: bool,
    in_changes: bool,
) -> bool {
    match (base, section) {
        (_, ReviewSection::FileTab) => false,
        (ReviewBase::BranchBase, ReviewSection::Changes) => in_files,
        (ReviewBase::BranchBase, ReviewSection::Staged) => false,
        (ReviewBase::Head, ReviewSection::Staged) => in_staged,
        (ReviewBase::Head, ReviewSection::Changes) => in_changes,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewPresentation {
    Image,
    Binary,
    Text,
}

impl ReviewPresentation {
    pub fn initial_file_mode(
        original: &super::super::ReviewSideBytes,
        modified: &super::super::ReviewSideBytes,
    ) -> FileMode {
        FileMode::for_change(original != modified)
    }

    pub fn classify(
        path: &str,
        binary_attribute: bool,
        original: &super::super::ReviewSideBytes,
        modified: &super::super::ReviewSideBytes,
    ) -> Self {
        if ReviewBlobContentType::image_from_path(path).is_some() {
            Self::Image
        } else if binary_attribute
            || review_bytes_are_binary(original)
            || review_bytes_are_binary(modified)
        {
            Self::Binary
        } else {
            Self::Text
        }
    }
}

pub fn review_bytes_are_binary(bytes: &super::super::ReviewSideBytes) -> bool {
    match bytes {
        super::super::ReviewSideBytes::Present(bytes) => {
            bytes.iter().take(8192).any(|byte| *byte == 0) || std::str::from_utf8(bytes).is_err()
        }
        super::super::ReviewSideBytes::Missing => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewLimitReason {
    FileSize,
    LineCount,
    HunkCount,
    Tokenization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewBlobContentType {
    is_image: bool,
}

impl ReviewBlobContentType {
    pub fn from_path(path: &str) -> Self {
        match path
            .rsplit('.')
            .next()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("bmp") | Some("svg")
            | Some("webp") | Some("ico") | Some("tiff") | Some("tif") | Some("avif") => {
                Self::image()
            }
            _ => Self::binary(),
        }
    }

    pub fn image_from_path(path: &str) -> Option<Self> {
        let content_type = Self::from_path(path);
        content_type.is_image().then_some(content_type)
    }

    pub fn is_image(self) -> bool {
        self.is_image
    }

    fn image() -> Self {
        Self { is_image: true }
    }

    fn binary() -> Self {
        Self { is_image: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewThresholds {
    pub max_file_size_bytes: u64,
    pub max_line_count: usize,
    pub max_hunk_count: usize,
    pub max_tokenization_chars: usize,
    pub max_tokenization_lines: usize,
}

impl Default for ReviewThresholds {
    fn default() -> Self {
        Self {
            max_file_size_bytes: 1_048_576,
            max_line_count: 5_000,
            max_hunk_count: 300,
            max_tokenization_chars: 100_000,
            max_tokenization_lines: 5_000,
        }
    }
}

impl ReviewThresholds {
    pub fn file_size_limit(self, size_bytes: u64) -> Option<ReviewLimitReason> {
        (size_bytes > self.max_file_size_bytes).then_some(ReviewLimitReason::FileSize)
    }

    pub fn line_count_limit(self, line_count: usize) -> Option<ReviewLimitReason> {
        (line_count > self.max_line_count).then_some(ReviewLimitReason::LineCount)
    }

    pub fn hunk_count_limit(self, hunk_count: usize) -> Option<ReviewLimitReason> {
        (hunk_count > self.max_hunk_count).then_some(ReviewLimitReason::HunkCount)
    }

    pub fn tokenization_limit(
        self,
        char_count: usize,
        line_count: usize,
    ) -> Option<ReviewLimitReason> {
        (char_count > self.max_tokenization_chars || line_count > self.max_tokenization_lines)
            .then_some(ReviewLimitReason::Tokenization)
    }
}

#[cfg(test)]
#[path = "review_test.rs"]
pub(crate) mod review_tests;
