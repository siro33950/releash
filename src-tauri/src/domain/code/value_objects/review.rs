//! Review read model の表示判定に使う値オブジェクト。

use crate::domain::code::CodeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewSection {
    Changes,
    Staged,
}

impl ReviewSection {
    pub fn parse(value: &str) -> Result<Self, CodeError> {
        match value {
            "changes" => Ok(Self::Changes),
            "staged" => Ok(Self::Staged),
            other => Err(CodeError::Rule(format!("invalid review section: {other}"))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Changes => "changes",
            Self::Staged => "staged",
        }
    }

    pub fn is_staged(self) -> bool {
        matches!(self, Self::Staged)
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
mod tests {
    use super::*;

    #[test]
    fn thresholds_are_inclusive_at_boundary_and_limit_above_it() {
        let thresholds = ReviewThresholds::default();

        assert_eq!(
            thresholds.file_size_limit(thresholds.max_file_size_bytes),
            None
        );
        assert_eq!(
            thresholds.file_size_limit(thresholds.max_file_size_bytes + 1),
            Some(ReviewLimitReason::FileSize)
        );
        assert_eq!(thresholds.line_count_limit(thresholds.max_line_count), None);
        assert_eq!(
            thresholds.line_count_limit(thresholds.max_line_count + 1),
            Some(ReviewLimitReason::LineCount)
        );
        assert_eq!(thresholds.hunk_count_limit(thresholds.max_hunk_count), None);
        assert_eq!(
            thresholds.hunk_count_limit(thresholds.max_hunk_count + 1),
            Some(ReviewLimitReason::HunkCount)
        );
        assert_eq!(
            thresholds.tokenization_limit(thresholds.max_tokenization_chars, 1),
            None
        );
        assert_eq!(
            thresholds.tokenization_limit(thresholds.max_tokenization_chars + 1, 1),
            Some(ReviewLimitReason::Tokenization)
        );
    }

    #[test]
    fn review_base_and_section_reject_unknown_values() {
        assert_eq!(ReviewBase::parse("head").unwrap(), ReviewBase::Head);
        assert_eq!(
            ReviewBase::parse("branch-base").unwrap(),
            ReviewBase::BranchBase
        );
        assert!(ReviewBase::parse("main").is_err());

        assert_eq!(
            ReviewSection::parse("changes").unwrap(),
            ReviewSection::Changes
        );
        assert_eq!(
            ReviewSection::parse("staged").unwrap(),
            ReviewSection::Staged
        );
        assert!(ReviewSection::parse("unstaged").is_err());
    }

    #[test]
    fn review_blob_content_type_classifies_images_without_mime() {
        let png = ReviewBlobContentType::from_path("assets/LOGO.PNG");
        assert!(png.is_image());

        let svg = ReviewBlobContentType::image_from_path("icons/app.svg").unwrap();
        assert!(svg.is_image());

        let binary = ReviewBlobContentType::from_path("archive.bin");
        assert!(!binary.is_image());
        assert_eq!(ReviewBlobContentType::image_from_path("archive.bin"), None);
    }
}
