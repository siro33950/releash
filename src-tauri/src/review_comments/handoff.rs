//! Review thread → Active AgentChat session への handoff メッセージ整形。
//!
//! spec issues-1022 "Thread handoff contract":
//! Desktop UI から「現在の Agent との対話に Thread を共有する」操作のメッセージ本文を
//! Rust 側で組み立て、UI はそれを active な AgentChat session の入力として送信する。
//! メッセージ整形は Rust が owner であり、フロントエンドは本文の作成を行わない。
//! Agent は受信メッセージから既存 `releash review` CLI で本文・履歴を自律取得する
//! (ライブ型: spec "Source of truth は CLI / usecase" 原則と整合)。

use super::ReviewThread;

/// Diff Thread → Active AgentChat session 共有メッセージを組み立てる。
///
/// 出力は「内容を確認してください」という最小限の指示と、取得用 CLI コマンド 1 行で構成する。
/// thread の本文・対象 file / lines / state / worktree などはすべて CLI 応答に含まれるため、
/// メッセージ本文には埋め込まない。Agent は受信メッセージから CLI を実行して必要な情報を
/// 取得する (ライブ型: spec "Source of truth は CLI / usecase" 原則)。Comment 追記 /
/// Resolve など以降の行動指示はユーザーが必要に応じて自分の言葉で追記する想定。
///
/// `$RELEASH_SESSION_ID` 環境変数 (spec "Agent process environment contract") から
/// session id を取り出して `--session-id` に渡す。worktree も session から解決される
/// (spec design.md "CLI contract")。
///
/// `releash_alias` 引数は build profile に応じた CLI 実行名 (`releash` / `releash-dev`) を
/// 表す。Agent process の `PATH` には `path_aliases` の wrapper が `alias_name_for_profile`
/// の名前でだけ登録されるため、ここをリテラル化すると Dev 環境 (`releash-dev`) で Agent が
/// CLI 解決に失敗する。呼び出し側 (Tauri command controller) で
/// `alias_name_for_profile(BuildProfile::current())` を解決して渡すこと。pure 関数性を保つ
/// ため、本 builder では `cfg!(debug_assertions)` 等の build profile 依存を持たない。
pub fn build_review_thread_handoff_message(releash_alias: &str, thread: &ReviewThread) -> String {
    format!(
        "以下のスレッドの内容を確認してください。\n\n{releash_alias} review get --session-id \"$RELEASH_SESSION_ID\" {}",
        thread.id
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review_comments::{
        ReviewActorDto, ReviewActorKind, ReviewTarget, ReviewThread, ReviewThreadState,
    };

    fn sample_thread() -> ReviewThread {
        ReviewThread {
            id: "thread-1234".to_string(),
            worktree_name: "feat-issues-1022".to_string(),
            author: ReviewActorDto {
                kind: ReviewActorKind::Human,
                backend_id: None,
                model: None,
                display_name: "Human".to_string(),
            },
            target: ReviewTarget {
                file_path: None,
                line_number: None,
                end_line: None,
            },
            state: ReviewThreadState::Open,
            comments: vec![],
            resolve: None,
            created_at: 0.0,
            updated_at: 0.0,
            version: 1,
            can_resolve: false,
        }
    }

    /// Production 環境 (alias = `releash`) では、最小限の指示と CLI コマンドだけが含まれる。
    /// Agent は CLI を実行して thread の本文・file / lines / state / worktree などを
    /// すべて自律取得する設計のため、メッセージ本文には参照情報を埋め込まない。
    #[test]
    fn handoff_message_for_production_alias_contains_only_instruction_and_cli() {
        let thread = sample_thread();
        let message = build_review_thread_handoff_message("releash", &thread);
        assert_eq!(
            message,
            "以下のスレッドの内容を確認してください。\n\nreleash review get --session-id \"$RELEASH_SESSION_ID\" thread-1234"
        );
    }

    /// spec issues-1022 Follow-up: Development 環境 (alias = `releash-dev`) では、
    /// メッセージ内のコマンドが `releash-dev` を使う。`path_aliases` の wrapper は
    /// `releash-dev` 名でしか PATH に登録されないため、この alias 名が反映されないと
    /// Dev 環境で Agent が CLI を呼べない。
    #[test]
    fn handoff_message_for_development_alias_uses_releash_dev() {
        let thread = sample_thread();
        let message = build_review_thread_handoff_message("releash-dev", &thread);
        assert_eq!(
            message,
            "以下のスレッドの内容を確認してください。\n\nreleash-dev review get --session-id \"$RELEASH_SESSION_ID\" thread-1234"
        );
    }
}
