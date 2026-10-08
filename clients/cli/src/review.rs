use clap::Subcommand;
#[derive(Subcommand, Debug)]
pub enum ReviewSubcommand {
    /// review Thread 一覧を表示する。
    List {
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        file: Option<String>,
        #[arg(long)]
        state: Option<String>,
        #[arg(long, requires = "session_id")]
        author: Option<String>,
        #[arg(long, requires = "session_id")]
        unread: Option<String>,
        #[arg(long = "thread-id")]
        thread_id: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// review Thread 詳細を表示する。
    Get {
        thread_id: String,
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        json: bool,
    },
    /// 初回 Comment とともに review Thread を作成する。
    Create {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        content: String,
        #[arg(long)]
        file: Option<String>,
        #[arg(long)]
        line: Option<u32>,
        #[arg(long)]
        end_line: Option<u32>,
        #[arg(long)]
        json: bool,
    },
    /// open Thread に Comment を追記する。
    Comment {
        thread_id: String,
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        content: String,
        #[arg(long)]
        json: bool,
    },
    /// 作成者 Agent として open Thread を resolve する。
    Resolve {
        thread_id: String,
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        outcome: String,
        #[arg(long)]
        summary: String,
        #[arg(long)]
        json: bool,
    },
    /// Thread 履歴を表示する。
    History {
        thread_id: String,
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        json: bool,
    },
}
