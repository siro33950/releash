//! code ドメインのドメインサービス（純粋ロジック）。
//!
//! diff の patch 生成、ディレクトリ一覧の判定、ファイル一覧の絞り込みを集約する。
//! いずれも git2 / ファイル I/O に依存しない。diff バッファの
//! 計算（git2 依存）は `DiffComputer`（gateway）が担い、本サービスは生成済みの
//! `Hunk` を入力に受け取る。

pub mod directory;
pub mod file_hunk;
pub mod filtered_files;
