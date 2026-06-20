//! code ドメインの外部リソース抽象（trait）。
//!
//! git2・ファイル I/O 等の具体実装は知らない。具体実装は
//! `adaptor/gateway/code/` に配置する。git2 のブロッキング呼び出しは gateway 内で
//! 同期的に行い、非同期境界（`spawn_blocking`）は controller 層で被せる方針のため、
//! 各 trait のメソッドは同期シグネチャで定義する。

use super::error::CodeError;
use super::value_objects::{Hunk, MentionReference};

/// 各リビジョン時点のファイル内容参照（テキスト／バイナリ Base64）。
pub trait FileContentRepository: Send + Sync {
    fn file_at_ref(&self, file_path: &str, git_ref: &str) -> Result<String, CodeError>;
    fn binary_file_at_ref(&self, file_path: &str, git_ref: &str) -> Result<String, CodeError>;
    /// `base_commit_oid` は usecase が解決済みの base コミット OID(hex)。`None` は
    /// detached / base 未設定で HEAD コミットにフォールバックする。
    fn file_at_branch_base(
        &self,
        file_path: &str,
        base_commit_oid: Option<&str>,
    ) -> Result<String, CodeError>;
    fn binary_file_at_branch_base(
        &self,
        file_path: &str,
        base_commit_oid: Option<&str>,
    ) -> Result<String, CodeError>;
    fn staged_content(&self, file_path: &str) -> Result<String, CodeError>;
    fn binary_staged_content(&self, file_path: &str) -> Result<String, CodeError>;
}

/// 差分の Approve（staging）に関わる index 書き込み操作。
pub trait StagingRepository: Send + Sync {
    fn stage(&self, repo_path: &str, paths: Vec<String>) -> Result<(), CodeError>;
    fn unstage(&self, repo_path: &str, paths: Vec<String>) -> Result<(), CodeError>;
    fn stage_hunk(&self, repo_path: &str, patch: &str) -> Result<(), CodeError>;
    fn unstage_hunk(&self, repo_path: &str, patch: &str) -> Result<(), CodeError>;
}

/// 2 つのテキストバッファの差分を hunk 列として計算する。
///
/// 差分アルゴリズム自体（git2 依存）を gateway へ閉じ込めるための抽象。
/// hunk 区切り（change group）や patch 生成といった後段の純粋ロジックは
/// ドメインサービス（`services::hunk`）が担う。
pub trait DiffComputer: Send + Sync {
    fn diff_buffers(&self, original: &str, modified: &str, file_path: Option<&str>) -> Vec<Hunk>;
}

/// 現在ブランチのベースブランチ名解決の抽象。
///
/// ベースブランチ名の解決ルール（per-branch override → global → default branch）自体は
/// `repository` ドメインが所有する。`code` ドメインの branch diff / branch base 参照は
/// merge-base 計算のために base 名を必要とするため、この port 越しに名前だけを受け取り、
/// 解決ルールの実装は持たない。`path_hint` は repo パスまたはファイルパス。
/// detached HEAD / unborn / 解決不可は `None`。
pub trait BranchBaseResolver: Send + Sync {
    /// 現在ブランチの base 名を解決する（ref 実在の検証はしない）。merge-base 計算の
    /// 入力に用い、ref の存在検証・error は後段の gateway（`FileContentRepository`）が担う。
    fn resolve_base_branch_name(&self, path_hint: &str) -> Result<Option<String>, CodeError>;

    /// 現在ブランチの実効 base 名を解決する（解決した base が local / remote の ref として
    /// 実在し、merge-base が計算できる場合のみ `Some`）。外部入口（agent bridge）が
    /// 「実在する base 名」だけを必要とする用途向け。
    fn resolve_effective_base_branch_name(
        &self,
        path_hint: &str,
    ) -> Result<Option<String>, CodeError>;

    /// 解決済み base 名の ref を解決し、base コミット OID(hex) を返す。ref 実在検証
    /// （local → remote）は repository ドメインが所有し、後段の gateway は本 OID と現在
    /// HEAD から merge-base を計算する。ref 不在は `None`。
    fn resolve_base_commit_oid(
        &self,
        path_hint: &str,
        base_name: &str,
    ) -> Result<Option<String>, CodeError>;
}

/// ファイルメンションの候補列挙（worktree 配下の .gitignore 準拠ファイル一覧）と
/// 参照解決（メンション先ファイルを `<file_context>` ブロックとして本文先頭へ前置）。
pub trait MentionRepository: Send + Sync {
    fn list_mentionable_files(
        &self,
        worktree_path: &str,
        query: &str,
    ) -> Result<Vec<String>, CodeError>;

    /// 構造化メンション参照を解決し、本文先頭へ file_context を前置した文字列を返す。
    /// メンションが空、または解決対象が 1 件も読めない場合は本文を変更せず返す。
    /// パストラバーサル等の拒否はエラーとして返し、フォールバック方針は usecase が決める。
    fn resolve_mentions(
        &self,
        worktree_path: &str,
        content: &str,
        mentions: &[MentionReference],
    ) -> Result<String, CodeError>;
}
