//! code ドメインの値オブジェクト。
//!
//! diff 計算の表現（`Hunk`、`DiffLine`）と、review の比較基準と区分、ファイル一覧の絞り込みの条件と結果を保持する。
//! いずれも純粋なデータであり、外部リソースに依存しない。

pub mod diff_line;
pub mod file_list;
pub mod hunk;
pub mod review;

pub use diff_line::DiffLine;
pub use hunk::Hunk;
pub use review::ReviewBase;
