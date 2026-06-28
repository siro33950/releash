# Requirements

対象 Issue: #1134「[impl] cli/mod.rs split」

マイルストーン: [12] クリーンアーキテクチャ移行

依存関係:
- Depends on: #1132（review CLI 部分。`review_comments` の clean architecture 移行）
- Blocks: #878（final sweep）

## Type

リファクタリング（実装 ISSUE）。user-visible な CLI behavior を変えずに、肥大化した `src-tauri/src/cli/mod.rs` を subcommand 単位の private module に分割し、公開 Rust 境界 `cli::run()` を維持する。

## 背景と目的

### 背景

- バックエンドは clean architecture へ段階移行中であり、ロジックは layer（`domain` / `usecase` / `adaptor` / `infrastructure`）へ寄せる方針になっている。
- 現状確認（リポジトリ事実）: `src-tauri/src/cli/mod.rs` は単一ファイルで約 3648 行あり、CLI の全責務が一箇所に集中している。
  - 引数定義: `Cli` / `TopCommand`（`Workflow` / `Review`）、`WorkflowSubcommand`、`OutputSubcommand`、`ReviewSubcommand`（`mod.rs` の 54〜258 行付近）。
  - 公開 entrypoint: `pub fn run() -> i32`（265 行付近）、`pub fn render_long_help()`（259 行付近）。
  - workflow 系コマンド: `cmd_list` / `cmd_runs` / `cmd_status` / `cmd_logs` / `cmd_enqueue_pending` / approve / reject / abort 関連（463 行〜）。
  - output 系コマンド: `cmd_output_submit` / `cmd_output_validate` / `cmd_output_get` とその helper（1033〜1280 行付近）。
  - review 系コマンド: `cmd_review` と review thread / comment 表示・検証 helper（701〜1032 行付近）。
  - 共有 helper: `resolve_data_dir` 系・`CliError`・`truncate`・event formatting など。
  - CLI-only tests: 同ファイル末尾の `#[cfg(test)] mod tests`（1486 行〜、約 2000 行超）。
- `render_long_help()` は Agent backend 起動時の system_prompt へ append される単一ソースであり、clap derive 由来の help に追従する設計（#1022 / spec [09]）。この公開 API も維持対象である。

### 改善する状態

本要求は以下を解消することを目的とする。

1. **CLI module の責務肥大**: 引数 parse・出力 formatting・各コマンド実装・共有 helper・大量の test が単一 `mod.rs` に同居し、subcommand ごとの境界が読み取りにくい。
2. **公開境界の不明確化リスク**: 内部関数が多数 module 直下に並び、公開すべき境界（`run()` / `render_long_help()`）と内部実装の区別が構造上表現されていない。

### 目的

- `cli/mod.rs` を subcommand 単位の private module（例: `cli/workflow.rs` / `cli/review.rs` / `cli/output.rs`、および presentation / 共有 helper module）へ分割する。
- 公開 Rust 境界は `cli::run()`（および既存の `cli::render_long_help()`）に限定し、それ以外を crate 外へ公開しない。
- 各 CLI module は引数 parse と terminal output formatting を持ち、usecase / controller-safe boundary を呼ぶに留める。domain behavior・persistence・workflow execution・review/comment state transition を CLI module が所有しない構造を維持する。
- `mod.rs` は module wiring と `run()` 中心へ縮小する。

## スコープ

`src-tauri/src/cli/mod.rs` の責務別分割と、それに付随する CLI-only test の再配置に限定する。

### 対象コード

- `src-tauri/src/cli/mod.rs`
- 同ファイル内の CLI-only tests
- 旧 module への CLI 直接依存（import 経路の見直し）

### 責務範囲（分割方針）

- 公開 Rust 境界は `cli::run()` のまま維持する（既存公開の `render_long_help()` も維持する）。
- subcommand 単位で private module に分割する。想定例:
  - `cli/workflow.rs`（list / runs / status / logs / enqueue / approve / reject / abort / output 系）
  - `cli/review.rs`（review thread / comment コマンド）
  - `cli/output.rs` または workflow module 内（submit / validate / get）
  - `cli/format.rs` などの CLI presentation / 共有 helper（`CliError`・data dir 解決・整形 helper）
- CLI module は引数 parse と terminal output formatting を持ってよい。
- CLI module は usecase / controller-safe boundary を呼ぶ。
- CLI module は domain behavior、persistence、workflow execution behavior、review/comment state transition を所有しない。
- CLI-only tests は分割後の各 module 内（`#[cfg(test)] mod tests`）へ責務に合わせて再配置する。

## 非スコープ

- `review_comments` / `git_host` / `notion` の clean architecture 移行: #1132 / #985 / #986。
- root platform glue の移動: #1133。
- CLI 分割と無関係な dead code 削除（全体 usage audit が必要なもの）: #878。
- CLI command name や user-visible behavior の変更（コマンド体系・出力仕様・終了コードは変えない）。
- clap AST / 内部 subcommand enum を crate 外へ公開すること（内部 AST は非公開境界のまま維持する）。

## 要求事項

### R1. 公開 entrypoint の維持

- crate 外へ公開する CLI entrypoint は `cli::run()` に限定する。
- 既存の公開 API `cli::render_long_help()` の振る舞いとシグネチャを維持する。
- clap AST・内部 subcommand enum（`TopCommand` / `WorkflowSubcommand` / `OutputSubcommand` / `ReviewSubcommand`）を crate 外へ公開しない。

### R2. subcommand 単位の module 分割

- `cli/mod.rs` を subcommand / 責務単位の private module へ分割する。
- `cli/mod.rs` は module wiring と `run()`（および公開 re-export）中心に縮小されている。

### R3. layer 境界の維持

- CLI module は引数 parse と terminal output formatting のみを持つ。
- CLI module は usecase / controller-safe boundary を呼ぶ。
- CLI module は domain behavior・persistence・workflow execution・review/comment state transition を所有しない。
- CLI module が `crate::review_comments` / `crate::git_host` / `crate::notion` / `crate::protocol` / `crate::ws_server` / `crate::ws_bridge` を import していない。

### R4. CLI behavior 不変

- CLI command name・引数・出力（人間向け / JSON）・終了コードを変更しない。
- 既存 CLI behavior と CLI-only tests が維持されている（再配置はしてよいが、検証内容を弱めない）。

### R5. 品質ゲート

- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通ること。

## 受け入れ基準の概要

Issue 記載の完了条件に対応する。

- **AC1**: `src-tauri/src/cli/mod.rs` が module wiring と `run()` 中心に縮小されている（R2）。
- **AC2**: `cli::run()` 以外を公開 entrypoint にしていない（既存公開の `render_long_help()` は維持）（R1）。
- **AC3**: CLI が `crate::review_comments` / `crate::git_host` / `crate::notion` / `crate::protocol` / `crate::ws_server` / `crate::ws_bridge` を import していない（R3）。
- **AC4**: 既存 CLI behavior と tests が維持されている（R4）。
- **AC5**: `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る（R5）。

## 仮定

- **A1**: spec ディレクトリ名は既存命名規約（`issues-1132` / `issues-1133`）に合わせ `docs/specs/issues-1134` とする。
- **A2**: 本 ISSUE は実装 ISSUE であり、設計 doc だけでなく実コードの分割・再配置まで含む。
- **A3**: 現状の `cli/mod.rs` の crate import は既に `adaptor` / `usecase` / `domain` layer 経由であり、AC3 が列挙する旧 module（`crate::review_comments` 等）への直接 import は現時点で存在しない。本 ISSUE はこの状態を分割後も維持する（regression を作らない）ことを要件とする。
- **A4**: review CLI 部分は #1132 完了後の着手が望ましいとされるが、現状コードは既に `crate::domain::comment` / `crate::usecase::comment` を経由しており、#1132 由来の境界を前提に分割を進める。
- **A5**: 具体的な module 分割粒度（`output` を独立 module にするか workflow 配下に置くか、helper module の境界、test の各 module への割り当て）は design で確定する。
- **A6**: `render_long_help()` は clap derive 定義に追従する単一ソースであり、引数定義 struct/enum の配置を移しても long help 出力が現状と一致することを検証対象とする。
- **A7**: behavior 不変の検証は既存 CLI-only tests（`cargo test`）と、コマンド出力・終了コードが変わらないことの確認に依拠する。新規の網羅的 CLI テスト追加は本 ISSUE のスコープ外とする。

## Open Questions

なし。
