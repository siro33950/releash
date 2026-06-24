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
pub(crate) mod branch_diff;
pub(crate) mod diff_compute;
mod error;
pub(crate) mod file_content;
pub(crate) mod mention;
pub(crate) mod review_blob_url;
pub(crate) mod staging;

use git2::Repository;

use crate::domain::code::CodeError;

/// 解決済み base コミット OID(hex) と現在 HEAD から merge-base コミットを返す。
///
/// `base_commit_oid` が `None`（detached / base 未設定）の場合は HEAD コミットへ
/// フォールバックする。base 名 → ref → コミット OID の解決は `repository` ドメインが
/// 所有し、本関数は OID から merge-base 計算のみを担う（`file_content` / `branch_diff`
/// が共有し、ref 解決ロジックを重複実装しない）。
pub(crate) fn resolve_merge_base_commit<'a>(
    repo: &'a Repository,
    base_commit_oid: Option<&str>,
) -> Result<git2::Commit<'a>, CodeError> {
    let head = repo.head().map_err(|e| {
        if e.code() == git2::ErrorCode::UnbornBranch {
            CodeError::Rule("unborn branch: no commits yet".to_string())
        } else {
            CodeError::from(e)
        }
    })?;
    let current_oid = head
        .target()
        .ok_or_else(|| CodeError::Rule("HEAD has no target".to_string()))?;

    // base 未指定（detached / base 未設定）→ HEAD コミットにフォールバック。
    let base_oid = match base_commit_oid {
        Some(oid_hex) => git2::Oid::from_str(oid_hex)?,
        None => return Ok(repo.find_commit(current_oid)?),
    };

    let merge_base_oid = repo.merge_base(current_oid, base_oid)?;
    Ok(repo.find_commit(merge_base_oid)?)
}
