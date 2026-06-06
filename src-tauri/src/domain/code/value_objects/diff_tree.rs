/// diff ファイルツリー構築の入力エントリ（フラットなファイル一覧の 1 件）。
#[derive(Debug, Clone)]
pub struct DiffFileEntry {
    pub path: String,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
}

/// diff ファイルツリーのノード（ディレクトリまたはファイル）。
#[derive(Debug, Clone)]
pub struct DiffTreeNode {
    pub id: String,
    pub name: String,
    pub path: String,
    pub node_type: String,
    pub status: Option<String>,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub children: Vec<DiffTreeNode>,
}

/// 現在ファイルを基準としたファイルナビゲーション情報。
#[derive(Debug, Clone)]
pub struct FileNavigationResult {
    pub current_index: usize,
    pub total: usize,
    pub prev_file: Option<String>,
    pub next_file: Option<String>,
}
