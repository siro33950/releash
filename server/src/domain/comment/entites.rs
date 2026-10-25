use super::Error;

/// Workspace に属する会話と判断の履歴。
#[derive(Debug, Clone, PartialEq)]
pub struct Thread {
    pub id: String,
    pub worktree_name: String,
    pub target: Target,
    pub state: State,
    pub comments: Vec<Comment>,
    pub resolve: Option<Resolve>,
    pub created_at: f64,
    pub updated_at: f64,
    pub version: u64,
    pub can_resolve: bool,
}

impl Thread {
    /// 自身の位置、ID、本文と、後続の Comment の確認方法を含む引き渡し内容を決める。
    pub(crate) fn handoff_content(&self) -> String {
        todo!()
    }
}

/// Comment や解決結果を記録した主体の種別。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorKind {
    Human,
    Agent,
}

/// Comment や解決結果を記録した人または Agent。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Author {
    pub(crate) kind: AuthorKind,
    pub backend_id: Option<String>,
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub(crate) display_name: String,
}

impl Author {
    /// 人による発言や解決の作者を表す。
    pub fn human() -> Self {
        Self {
            kind: AuthorKind::Human,
            backend_id: None,
            model: None,
            session_id: None,
            display_name: "Human".to_string(),
        }
    }

    /// provider と Session に結び付く Agent の作者を表す。
    pub fn provider_agent(backend_id: String, session_id: Option<String>) -> Self {
        Self {
            kind: AuthorKind::Agent,
            display_name: backend_id.clone(),
            backend_id: Some(backend_id),
            model: None,
            session_id,
        }
    }

    /// 会話の参加者を同一とみなす識別子を決める。
    pub(crate) fn participant_key(&self) -> String {
        match self.kind {
            AuthorKind::Human => "human".to_string(),
            AuthorKind::Agent => format!(
                "agent:{}:{}",
                self.backend_id.as_deref().unwrap_or_default(),
                self.model.as_deref().unwrap_or_default()
            ),
        }
    }
}

/// Thread が未解決か解決済みかを表す状態。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum State {
    Open,
    Resolved,
}

/// Thread が対象にする差分の追加側または削除側。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineSide {
    Additions,
    Deletions,
}

/// Thread が対象にするファイルと差分上の位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub file_path: Option<String>,
    pub line_number: Option<u32>,
    pub end_line: Option<u32>,
    pub side: Option<LineSide>,
    pub end_side: Option<LineSide>,
}

/// Thread の本文または返信として記録された発言。
#[derive(Debug, Clone, PartialEq)]
pub struct Comment {
    pub id: String,
    pub thread_id: String,
    pub author: Author,
    pub content: String,
    pub created_at: f64,
}

/// Thread を解決した主体、結論、要約、時刻の記録。
#[derive(Debug, Clone, PartialEq)]
pub struct Resolve {
    pub actor: Author,
    pub outcome: String,
    pub summary: String,
    pub resolved_at: f64,
}

impl Target {
    /// 対象のパス、行範囲、行の側の指定が有効かを判定する。
    pub(crate) fn validate(&self) -> Result<(), Error> {
        todo!()
    }
}

impl State {
    /// open または resolved の状態指定を解釈する。
    pub fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "open" => Ok(Self::Open),
            "resolved" => Ok(Self::Resolved),
            _ => Err(Error::InvalidInput("Invalid review state".into())),
        }
    }
    /// 状態に対応する open または resolved の表記を返す。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Resolved => "resolved",
        }
    }
}
