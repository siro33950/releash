//! code ドメインの gateway 実装（domain trait / usecase port の具体実装）。
//!
//! git2・ファイルシステム・git CLI の呼び出しと、ドメイン型 ↔ 外部型の変換を内部に
//! 閉じる。これらを usecase / query service へ合成する DI 配線（composition root）は
//! controller の責務であり、本モジュールには置かない（[`crate::adaptor::controller::wiring`]）。
//!
//! MCP / backends など非 AppState エントリも、ファイル内容参照・base branch 名解決・
//! mention 解決を gateway 実装関数へ直接依存せず、composition root（`wiring`）が組み立てた
//! `CodeUsecase` の公開 API 経由で利用する。gateway は domain trait / usecase port の具体
//! 実装（`*Gateway` 構造体）に閉じる。

pub(crate) mod branch_base;
pub(crate) mod diff_compute;
pub(crate) mod error;
pub(crate) mod file_content;
pub(crate) mod staging;

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_helpers;

pub(crate) mod directory;
