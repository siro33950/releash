//! 外部システムのエラー → ドメインエラー（`CodeError`）への変換。
//!
//! ドメイン層は git2 / I/O 等の外部型に依存できないため、これらの `From` 実装は
//! gateway 層に置く。serialize 表現を移行前（`GitError` のプレーン文字列）と等価に
//! 保つため、いずれも元エラーの `Display` 文字列をそのまま `External` へ畳み込む。

use crate::domain::code::CodeError;

impl From<git2::Error> for CodeError {
    fn from(e: git2::Error) -> Self {
        CodeError::External(e.to_string())
    }
}

impl From<std::io::Error> for CodeError {
    fn from(e: std::io::Error) -> Self {
        CodeError::External(e.to_string())
    }
}

impl From<std::str::Utf8Error> for CodeError {
    fn from(e: std::str::Utf8Error) -> Self {
        CodeError::External(e.to_string())
    }
}

impl From<std::path::StripPrefixError> for CodeError {
    fn from(e: std::path::StripPrefixError) -> Self {
        CodeError::External(e.to_string())
    }
}
