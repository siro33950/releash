//! code ドメイン（ファイル内容参照・diff・diff_tree・branch_diff・hunk・patch・
//! staging・language・file_mention・visible/hidden range）。
//!
//! 純粋ロジック（hunk 区切り・patch 生成・language 判定・range 算出・mention の
//! 文字列処理・diff_tree 構築）はドメインサービス／値オブジェクトとして集約する。
//! 外部リソースに触れる責務（ファイル内容参照・diff バッファ計算・branch diff・
//! staging・mention のファイル走査）は trait（`repository.rs`）で抽象化し、具体実装は
//! `adaptor/gateway/code/` に閉じる。`docs/architecture/DOMAIN.md` 準拠で
//! 外部ライブラリ（git2・tokio・tauri・std I/O）を直接 `use` しない。

pub mod error;
#[allow(clippy::module_inception)]
pub mod repository;
pub mod services;
pub mod value_objects;

pub use error::CodeError;
pub use repository::{
    BranchBaseResolver, DiffComputer, FileContentRepository, MentionRepository, ReviewBlobSide,
    ReviewBlobUrlParams, ReviewBlobUrlProvider, ReviewSideBytes, ReviewSideMetadata,
    StagingRepository,
};
pub use value_objects::{
    ChangeGroup, DiffFileEntry, DiffTreeNode, HiddenRange, Hunk, MentionReference, ReviewBase,
    ReviewBlobContentType, ReviewLimitReason, ReviewSection, ReviewThresholds, VisibleBlock,
};
