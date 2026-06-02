//! 外部システムのエラー → ドメインエラー（`RepositoryError`）への変換。
//!
//! ドメイン層は git2 / I/O 等の外部型に依存できないため、これらの
//! `From` 実装は gateway 層に置く。serialize 表現を移行前と等価に保つため、
//! いずれも元エラーの `Display` 文字列をそのまま保持する。

use crate::domain::repository::RepositoryError;

impl From<git2::Error> for RepositoryError {
    fn from(e: git2::Error) -> Self {
        RepositoryError::External(e.to_string())
    }
}

impl From<std::io::Error> for RepositoryError {
    fn from(e: std::io::Error) -> Self {
        RepositoryError::External(e.to_string())
    }
}

impl From<std::str::Utf8Error> for RepositoryError {
    fn from(e: std::str::Utf8Error) -> Self {
        RepositoryError::External(e.to_string())
    }
}

impl From<std::path::StripPrefixError> for RepositoryError {
    fn from(e: std::path::StripPrefixError) -> Self {
        RepositoryError::External(e.to_string())
    }
}
