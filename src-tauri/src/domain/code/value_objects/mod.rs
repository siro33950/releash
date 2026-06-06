//! code ドメインの値オブジェクト。
//!
//! diff 計算の表現（`Hunk` / `ChangeGroup`）、range 表現（`HiddenRange` /
//! `VisibleBlock`）、diff_tree の表現（`DiffFileEntry` / `DiffTreeNode` /
//! `FileNavigationResult`）、メンション参照（`MentionReference`）を保持する。
//! いずれも純粋なデータであり、外部リソースに依存しない。
//!
//! いずれの値オブジェクトも serde 非依存（転送表現は usecase の DTO と controller
//! 入口の入力型が保持する）。`MentionReference` も同様に serde 非依存とし、agent /
//! backends / workflow / session は domain 型として受け渡し、転送表現は adaptor 側の
//! 入力型が保持する。

pub mod diff_tree;
pub mod hunk;
pub mod mention_reference;
pub mod range;

pub use diff_tree::{DiffFileEntry, DiffTreeNode, FileNavigationResult};
pub use hunk::{ChangeGroup, Hunk};
pub use mention_reference::MentionReference;
pub use range::{HiddenRange, VisibleBlock};
