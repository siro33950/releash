use serde::Serialize;

/// アプリ横断のエラー型。adaptor 層（Tauri コマンド / WebSocket ハンドラ）の
/// 戻り値で用いる。
///
/// フロント／リモートへ返却される serialize 表現は、移行前の `GitError`
/// （プレーン文字列）と等価であることを契約とする。そのため variant は
/// 文字列メッセージのみを保持し、`serialize_str` で文字列として返す。
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializeはプレーン文字列を返す() {
        // 移行前の GitError（プレーン文字列）と等価な観測表現の契約。
        // オブジェクトに包まず、メッセージそのものを JSON 文字列として返す。
        let err = AppError::new("リポジトリパスが設定されていません");
        let json = serde_json::to_string(&err).unwrap();
        assert_eq!(json, "\"リポジトリパスが設定されていません\"");
    }

    #[test]
    fn displayはメッセージそのもの() {
        let err = AppError::new("boom");
        assert_eq!(err.to_string(), "boom");
    }
}
