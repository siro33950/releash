//! code ドメインの値オブジェクト。
//!
//! diff 計算の表現（`Hunk` / `ChangeGroup`）、range 表現（`HiddenRange` /
//! `VisibleBlock`）、diff_tree の表現（`DiffFileEntry` / `DiffTreeNode` /
//! `FileNavigationResult`）、メンション参照（`MentionReference`）を保持する。
//! いずれも純粋なデータであり、外部リソースに依存しない。
//!
//! diff / hunk / range / diff_tree 系の値オブジェクトは serde 非依存（転送表現は
//! usecase の DTO と controller 入口の入力型が保持する）。`MentionReference` は
//! agent / backends / workflow / session が prompt 解決で利用する公開型のため serde を維持する。

pub mod diff_tree;
pub mod hunk;
pub mod mention_reference;
pub mod range;

pub use diff_tree::{DiffFileEntry, DiffTreeNode, FileNavigationResult};
pub use hunk::{ChangeGroup, Hunk};
pub use mention_reference::MentionReference;
pub use range::{HiddenRange, VisibleBlock};
