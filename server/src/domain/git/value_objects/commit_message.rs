use crate::domain::git::GitError;

/// commit メッセージ。前後の空白を除いた 1 文字以上の文字列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitMessage(String);

impl CommitMessage {
    /// 入力から commit メッセージを作る。前後の空白を除き、残りが無ければ commit メッセージではないので失敗する。
    pub fn new(input: &str) -> Result<Self, GitError> {
        let message = input.trim();
        if message.is_empty() {
            return Err(GitError::rule("Commit message is empty"));
        }
        Ok(Self(message.to_owned()))
    }

    /// commit メッセージの文字列を返す。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
#[path = "commit_message_test.rs"]
mod commit_message_tests;
