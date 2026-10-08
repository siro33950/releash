//! code ドメインのドメインサービス（純粋ロジック）。
//!
//! diff の hunk 区切り（change group）・patch 生成・visible/hidden range 算出・
//! Markdown 可視ブロック算出、diff_tree 構築・ナビゲーション、language 判定、
//! mention の文字列処理（fuzzy 一致・抜粋・XML エスケープ）を集約する。
//! いずれも git2 / ファイル I/O に依存しない純粋関数である。diff バッファの
//! 計算（git2 依存）は `DiffComputer`（gateway）が担い、本サービスは生成済みの
//! `Hunk` を入力に受け取る。

pub mod diff_tree;
pub mod hunk;
pub mod language;
pub mod markdown_diff;
