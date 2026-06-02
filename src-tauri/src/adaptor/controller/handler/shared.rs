//! WebSocket ハンドラ層の共有エラーヘルパー。
//!
//! `WsMessage` を組み立てるだけの汎用ヘルパーで、ws_server 固有ロジックには
//! 依存しない。応答エラー組み立ては handler 層の関心事のため、ここに置いて
//! adaptor/controller/handler と ws_server の双方から参照する（依存方向を
//! ws_server → adaptor の一方向に保ち、循環を生まない）。

use crate::protocol::*;

/// worktree 未選択時のエラー応答。
pub(crate) fn no_worktree_selected_error() -> WsMessage {
    WsMessage::Error(ErrorMsg {
        code: "NO_WORKTREE_SELECTED".to_string(),
        message: "Worktreeが選択されていません".to_string(),
    })
}

/// `spawn_blocking` の join 失敗時のエラー応答。
pub(crate) fn join_error_msg(e: tokio::task::JoinError) -> WsMessage {
    WsMessage::Error(ErrorMsg {
        code: "INTERNAL_ERROR".to_string(),
        message: format!("Task join error: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_worktree_selected_error() {
        let msg = no_worktree_selected_error();
        match msg {
            WsMessage::Error(e) => {
                assert_eq!(e.code, "NO_WORKTREE_SELECTED");
                assert!(!e.message.is_empty());
            }
            _ => panic!("Expected Error variant"),
        }
    }

    #[test]
    fn test_join_error_msg() {
        // JoinError は直接生成できないため、panic した task の join 結果から得る。
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(async {
            tokio::task::spawn(async { panic!("boom") })
                .await
                .unwrap_err()
        });
        let msg = join_error_msg(err);
        match msg {
            WsMessage::Error(e) => {
                assert_eq!(e.code, "INTERNAL_ERROR");
                assert!(e.message.contains("Task join error"));
            }
            _ => panic!("Expected Error variant"),
        }
    }
}
