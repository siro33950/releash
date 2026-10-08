//! code ドメインの値オブジェクト。
//!
//! diff 計算の表現（`Hunk` / `ChangeGroup`）、range 表現（`HiddenRange` /
//! `VisibleBlock`）、diff_tree の表現（`DiffFileEntry` / `DiffTreeNode` /
//! `FileNavigationResult`）を保持する。
//! いずれも純粋なデータであり、外部リソースに依存しない。
//!
//! いずれの値オブジェクトも serde 非依存（転送表現は usecase の DTO と controller
//! 入口の入力型が保持する）。

pub mod diff_tree;
pub mod hunk;
pub mod markdown_diff;
pub mod range;
pub mod review;

pub use diff_tree::{DiffFileEntry, DiffTreeNode, FileNavigationResult};
pub use hunk::{ChangeGroup, Hunk};
pub use markdown_diff::{
    DiffRange, DiffRangeKind, DiffSide, InlineChunk, InlineChunkKind, SplitRow, SplitRowKind,
};
pub use range::{HiddenRange, VisibleBlock};
pub use review::{
    ReviewBase, ReviewBlobContentType, ReviewLimitReason, ReviewSection, ReviewThresholds,
};
