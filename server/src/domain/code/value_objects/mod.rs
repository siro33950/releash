//! code ドメインの値オブジェクト。
//!
//! diff 計算の表現（`Hunk`）と、review の比較基準と区分を保持する。
//! いずれも純粋なデータであり、外部リソースに依存しない。

pub mod hunk;
pub mod review;

pub use hunk::Hunk;
pub use review::ReviewBase;
