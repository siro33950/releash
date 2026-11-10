//! code ドメインのドメインサービス（純粋ロジック）。
//!
//! diff の patch 生成とディレクトリ一覧の判定を集約する。
//! いずれも git2 / ファイル I/O に依存しない純粋関数である。diff バッファの
//! 計算（git2 依存）は `DiffComputer`（gateway）が担い、本サービスは生成済みの
//! `Hunk` を入力に受け取る。

pub mod directory;
pub mod file_hunk;
