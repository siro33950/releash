/// GitHub の PR に書かれた文章の書き手。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrCommentAuthor {
    /// 書き手の名前。
    pub login: String,
    /// 書き手が bot か。
    pub is_bot: bool,
}

/// GitHub の PR に書かれた文章の 1 件。PR 全体への Comment か、レビューの本文である。
/// Thread の Comment とは別のものである。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrComment {
    pub author: PrCommentAuthor,
    /// 書いた時刻（UNIX 秒）。レビューの本文では、レビューを提出した時刻。
    pub written_at: u64,
    /// 本文。Markdown。
    pub body: String,
}

/// PR の面の COMMENTS に並ぶもの。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrConversation(Vec<PrComment>);

impl PrConversation {
    /// COMMENTS に並ぶものと、その順を決める。
    /// PR 全体への Comment の全部と、本文が空でないレビューの本文を混ぜ、書いた時刻の古い順に並べる。
    /// 本文が空のレビューは入れない。
    pub fn new(comments: Vec<PrComment>, review_bodies: Vec<PrComment>) -> Self {
        let mut entries = comments;
        entries.extend(
            review_bodies
                .into_iter()
                .filter(|review| !review.body.is_empty()),
        );
        entries.sort_by_key(|entry| entry.written_at);
        Self(entries)
    }

    /// 並んだ順の項目。
    pub fn entries(&self) -> &[PrComment] {
        &self.0
    }
}

/// これから投稿する、PR 全体への Comment の本文。空でない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrCommentText(String);

impl PrCommentText {
    /// 投稿できる本文かを決める。空の本文は `PrCommentError::Empty` で拒む。
    pub fn new(text: String) -> Result<Self, PrCommentError> {
        if text.is_empty() {
            Err(PrCommentError::Empty)
        } else {
            Ok(Self(text))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// PR 全体への Comment を投稿できなかった理由。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrCommentError {
    /// 本文が空である。
    #[error("Comment の本文が空です。")]
    Empty,
}

#[cfg(test)]
#[path = "pr_comment_test.rs"]
mod pr_comment_tests;
