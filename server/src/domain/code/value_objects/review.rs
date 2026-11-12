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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewContentSource {
    BranchBaseTip,
    Head,
    WorkingTree,
}

impl ReviewContentSource {
    pub fn select(side: super::super::ReviewBlobSide, base: ReviewBase) -> Self {
        use super::super::ReviewBlobSide as Side;
        match (base, side) {
            (ReviewBase::BranchBase, Side::Original) => Self::BranchBaseTip,
            (_, Side::Original) => Self::Head,
            (_, Side::Modified) => Self::WorkingTree,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewPresentation {
    Image,
    Binary,
    Text,
}

impl ReviewPresentation {
    pub fn initial_file_mode(matches: bool) -> FileMode {
        FileMode::for_change(!matches)
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

#[cfg(test)]
#[path = "review_test.rs"]
pub(crate) mod review_tests;
