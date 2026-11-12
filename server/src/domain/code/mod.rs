//! code ドメイン（ファイル内容参照・diff・hunk・staging）。
//!
//! 純粋ロジック（patch 生成・ディレクトリ一覧の判定）はドメインサービス／値オブジェクトとして集約する。
//! 外部リソースに触れる責務（ファイル内容参照・diff バッファ計算・staging）は trait（`repository.rs`）で抽象化し、具体実装は
//! `adaptor/gateway/code/` に閉じる。`docs/architecture/DOMAIN.md` 準拠で
//! 外部ライブラリ（git2・tokio・std I/O）を直接 `use` しない。

pub mod error;
#[allow(clippy::module_inception)]
pub mod repository;
pub mod services;
pub mod value_objects;

pub use error::CodeError;
pub use repository::{
    BranchBaseResolver, DiffComputer, FileContentRepository, ReviewBlobSide, ReviewSideBytes,
    ReviewSideMetadata, StagingRepository, WorktreeEntries,
};
pub use value_objects::{Hunk, ReviewBase};
