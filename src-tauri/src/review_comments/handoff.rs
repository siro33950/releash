//! Review thread → Active AgentChat session への handoff メッセージ整形。
//!
//! spec issues-1022 "Thread handoff contract":
//! Desktop UI から「現在の Agent との対話に Thread を共有する」操作のメッセージ本文を
//! Rust 側で組み立て、UI はそれを active な AgentChat session の入力として送信する。
//! メッセージ整形は Rust が owner であり、フロントエンドは本文の作成を行わない。
//! Agent は受信メッセージから既存 `releash review` CLI で本文・履歴を自律取得する
//! (ライブ型: spec "Source of truth は CLI / usecase" 原則と整合)。

use super::{ReviewThread, ReviewThreadState};

/// Diff Thread → Active AgentChat session 共有メッセージを組み立てる。
///
/// 含める参照情報:
/// - thread_id: Agent が CLI で thread を取得するためのキー
/// - worktree: 対象 worktree 識別子 (`releash` CLI が session 経由で path を解決するため
///   識別子のみで十分。spec design.md "CLI contract": session が worktree を解決する)
/// - file: 対象ファイルパス (位置不依存 thread の場合は省略)
/// - lines: 対象行範囲 (単一行は単独数値、複数行は `start-end`)
/// - state: open / resolved
///
/// メッセージ末尾には推奨 CLI コマンド (`releash review get`) を実行例として埋め込み、
/// `$RELEASH_SESSION_ID` 環境変数 (spec "Agent process environment contract") から
/// session id を取り出して `--session-id` に渡す形を案内する。
pub fn build_review_thread_handoff_message(worktree_name: &str, thread: &ReviewThread) -> String {
    let mut lines = Vec::new();
    lines.push("レビュースレッドへの対応を依頼します。".to_string());
    lines.push(String::new());
    lines.push(format!("- thread_id: {}", thread.id));
    lines.push(format!("- worktree: {worktree_name}"));
    if let Some(file) = thread.target.file_path.as_deref() {
        lines.push(format!("- file: {file}"));
    }
    if let Some(range) = format_line_range(thread.target.line_number, thread.target.end_line) {
        lines.push(format!("- lines: {range}"));
    }
    let state_label = match thread.state {
        ReviewThreadState::Open => "open",
        ReviewThreadState::Resolved => "resolved",
    };
    lines.push(format!("- state: {state_label}"));
    lines.push(String::new());
    lines.push("本文と現在の Stance / Comment を取得して内容を確認してください:".to_string());
    lines.push(String::new());
    lines.push(format!(
        "    releash review get --session-id \"$RELEASH_SESSION_ID\" {}",
        thread.id
    ));
    lines.push(String::new());
    lines.push(
        "必要に応じて Comment 追記・Stance 表明・Resolve を行ってください (権限・状態は CLI の応答に従ってください)。"
            .to_string(),
    );
    lines.join("\n")
}

fn format_line_range(start: Option<u32>, end: Option<u32>) -> Option<String> {
    match (start, end) {
        (Some(s), Some(e)) if s != e => Some(format!("{s}-{e}")),
        (Some(s), _) => Some(s.to_string()),
        (None, Some(e)) => Some(e.to_string()),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review_comments::{
        ReviewActorDto, ReviewActorKind, ReviewStanceValue, ReviewTarget, ReviewThread,
        ReviewThreadState,
    };

    fn sample_thread(target: ReviewTarget, state: ReviewThreadState) -> ReviewThread {
        ReviewThread {
            id: "thread-1234".to_string(),
            worktree_name: "feat-issues-1022".to_string(),
            author: ReviewActorDto {
                kind: ReviewActorKind::Human,
                backend_id: None,
                model: None,
                display_name: "Human".to_string(),
            },
            target,
            state,
            comments: vec![],
            stances: vec![],
            resolve: None,
            created_at: 0.0,
            updated_at: 0.0,
            version: 1,
            can_resolve: false,
            my_stance: ReviewStanceValue::None,
        }
    }

    #[test]
    fn handoff_message_for_positioned_thread_includes_file_and_line_range() {
        let thread = sample_thread(
            ReviewTarget {
                file_path: Some("src/components/panels/DiffInlineComment.tsx".to_string()),
                line_number: Some(82),
                end_line: Some(139),
            },
            ReviewThreadState::Open,
        );
        let message = build_review_thread_handoff_message("feat-issues-1022", &thread);
        assert!(
            message.contains("thread-1234"),
            "missing thread_id: {message}"
        );
        assert!(
            message.contains("- worktree: feat-issues-1022"),
            "missing worktree identifier: {message}"
        );
        assert!(
            message.contains("src/components/panels/DiffInlineComment.tsx"),
            "missing file path: {message}"
        );
        assert!(message.contains("82-139"), "missing line range: {message}");
        assert!(
            message.contains("- state: open"),
            "missing state: {message}"
        );
        assert!(
            message.contains("releash review get"),
            "missing CLI hint: {message}"
        );
        assert!(
            message.contains("$RELEASH_SESSION_ID"),
            "must hint environment variable usage: {message}"
        );
    }

    #[test]
    fn handoff_message_for_position_less_thread_omits_file_and_lines() {
        let thread = sample_thread(
            ReviewTarget {
                file_path: None,
                line_number: None,
                end_line: None,
            },
            ReviewThreadState::Open,
        );
        let message = build_review_thread_handoff_message("feat-issues-1022", &thread);
        assert!(
            !message.contains("- file:"),
            "must omit file line for position-less thread: {message}"
        );
        assert!(
            !message.contains("- lines:"),
            "must omit lines line for position-less thread: {message}"
        );
        assert!(message.contains("thread-1234"));
        assert!(message.contains("releash review get"));
    }

    #[test]
    fn handoff_message_for_single_line_thread_renders_single_line_number() {
        let thread = sample_thread(
            ReviewTarget {
                file_path: Some("a.rs".to_string()),
                line_number: Some(42),
                end_line: Some(42),
            },
            ReviewThreadState::Open,
        );
        let message = build_review_thread_handoff_message("wt", &thread);
        assert!(
            message.contains("- lines: 42"),
            "single-line range must be a single number: {message}"
        );
        assert!(!message.contains("42-42"));
    }

    #[test]
    fn handoff_message_for_resolved_thread_marks_state_as_resolved() {
        let thread = sample_thread(
            ReviewTarget {
                file_path: Some("a.rs".to_string()),
                line_number: Some(1),
                end_line: Some(1),
            },
            ReviewThreadState::Resolved,
        );
        let message = build_review_thread_handoff_message("wt", &thread);
        assert!(
            message.contains("- state: resolved"),
            "missing resolved state label: {message}"
        );
    }
}
