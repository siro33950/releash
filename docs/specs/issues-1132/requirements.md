# Requirements

## Type

clean architecture 配置への移行（リファクタリング / 構造移行）。外部から観測可能な振る舞い（CLI / Tauri command / 永続化フォーマット）は変えない。

対象 Issue: #1132（[impl] review_comments / comment migration）

種別: Implementation ISSUE（親 ISSUE ではない）。comment 移行の正本 ISSUE は本 ISSUE とする。
マイルストーン: [12] クリーンアーキテクチャ移行

## 背景と目的

### 背景

現状、review comment（review thread / comment）のロジックは、layer 区分を持たない `src-tauri/src/review_comments/` 配下に集約されている（実コードで確認）:

- `review_comments/mod.rs`（約 2,655 行）: entity / value object（`ReviewThread`・`ReviewComment`・`ReviewActor`・`ReviewTarget`・`ReviewThreadFilter`・`ReviewHistoryEntry` 等）、event 表現（`ReviewEvent`）、state transition / validation / projection（`ThreadAccumulator`・`project_threads`・`apply_filter`）、error type（`ReviewError`）、file-backed event store（`ReviewCommentStore`・`ReviewPersistenceGateway`）、lock / JSON serialization / atomic replace が **1 module に同居** している。
- `review_comments/commands.rs`（約 221 行）: 8 つの Tauri command（`list_review_threads` / `get_review_thread` / `create_review_thread` / `append_review_comment` / `resolve_review_thread` / `delete_review_thread` / `get_review_thread_history` / `build_review_thread_handoff`）。
- `review_comments/handoff.rs`（約 95 行）: review thread handoff 生成。
- `review_comments/watcher.rs`（約 239 行）: review state file の file watching。
- `cli/mod.rs`: `releash review ...` サブコマンドが `crate::review_comments::{...}` へ直接依存（型・`ReviewError` の map・`ReviewThread` 整形等）。

この配置は、CLAUDE.md / `docs/architecture/` が定める clean architecture（domain / usecase / adaptor / infrastructure）の層境界を持たず、domain ロジックが filesystem・Tauri・CLI・watcher と同居している。Milestone [12]（クリーンアーキテクチャ移行）は agent / session 等の他モジュールを順次移行しており、review comment はその残対象である。

### 目的

`review_comments` モジュールの責務を clean architecture の層へ再配置し、`src-tauri/src/review_comments/` を解消する。具体的には、domain ロジック（entity / state transition / validation / projection）を infrastructure 非依存の `domain/comment/` へ、application flow を `usecase/comment/` へ、file-backed event store を `adaptor/gateway/comment/` へ、Tauri command wrapper を `adaptor/controller/command/comment/` へ移し、CLI / Tauri 登録経路を新 usecase 境界へ接続する。これにより、comment の domain 判断が infrastructure / delivery layer から分離され、同じ backend-owned state を Tauri・CLI・将来の client surface から再利用できる状態にする。本移行は外部から観測可能な振る舞い（CLI 出力契約・Tauri command の I/O・永続化フォーマット）を変えない。

## スコープ

- **domain 層の抽出**（`domain/comment/`）
  - Thread / Comment の entity・value object（`ReviewThread`・`ReviewComment`・`ReviewActor`・`ReviewTarget`・`ReviewResolveInfo`・`ReviewThreadState` 等）。
  - actor、target、filter、history event、state transition、validation、projection rule（`ThreadAccumulator` / `project_thread(s)` / `apply_filter` / `validate_*` / `ensure_*` / `is_unread_for_viewer` 等の純粋ロジック）。
  - comment 専用 error type（`ReviewError` / `ReviewErrorCode` 相当）。
  - Tauri / filesystem / watcher / CLI / WebSocket に依存しないこと。
- **usecase 層の抽出**（`usecase/comment/`）
  - list / get / create / append / resolve / delete / history / handoff の application flow。
  - storage、time / id 生成（`now()` / `event_id()`）、watcher / notification 副作用が必要な箇所を port として定義し、usecase は port 経由で利用すること。
- **gateway 層の抽出**（`adaptor/gateway/comment/`）
  - 現 `ReviewCommentStore` / `ReviewPersistenceGateway` の file-backed event store 実装。
  - worktree 単位の lock、JSON serialization、atomic replace、破損ファイル・欠損ファイル処理。
  - usecase が定義した storage port を実装すること。
- **controller 層の抽出**（`adaptor/controller/command/comment/`）
  - 8 つの Tauri command wrapper と request / response mapping。
  - business behavior を持たず、usecase 呼び出しへの変換に閉じること。
- **watcher の再配置**（`infrastructure`）
  - `review_comments/watcher.rs` の file watching 実装を、gateway に自然に収まらない場合 `infrastructure` 配下へ置く。
- **CLI 接続の更新**
  - `cli/mod.rs` の `releash review ...` 呼び出し元を、新 usecase 境界へ接続するために必要な範囲のみ更新する。`crate::review_comments` への直接依存を解消する。
- **command 登録経路の更新**
  - `adaptor/controller/command/mod.rs`（command 登録）と `lib.rs` の `mod review_comments` を、新配置に合わせて更新・除去する。
- **テストの移設・整備**
  - domain / usecase test が正常系・エラー系の list / get / create / append / resolve / delete / history / handoff をカバーすること。
  - gateway test が永続化、lock、破損 / 欠損ファイルをカバーすること。

## 非スコープ

- GitHub PR review comment の取得（#985）。
- `cli/mod.rs` 全体の分割（#1134）。本 Issue は review 接続を新境界へ繋ぐのに必要な範囲のみ触る。なお #1134 の review CLI 分割部分は本 Issue を Blocks 先とする。
- WebSocket protocol 整理（#1130）。
- WebSocket server / broadcaster 整理（#1131）。
- 本移行と無関係な dead code 削除（#878。final sweep は Related）。
- 外部から観測可能な振る舞いの変更（CLI 出力契約、Tauri command の I/O 形状、永続化フォーマット / state file レイアウトの変更）。本移行は配置移動であり、機能追加・仕様変更を行わない。
- public 型・command 名のリネームを伴う API 仕様変更（移行に伴う module path 変更を除く）。

## 要求事項

- R1: comment の domain ロジック（entity / value object / actor / target / filter / history event / state transition / validation / projection / error type）が `domain/comment/` に置かれ、Tauri・filesystem・watcher・CLI・WebSocket のいずれにも依存しないこと。
- R2: list / get / create / append / resolve / delete / history / handoff の application flow が `usecase/comment/` に置かれ、storage・time / id 生成・watcher / notification 等の副作用を port 経由で扱うこと（domain は infrastructure 非依存、ロジックは Rust 側に置く: `.claude/rules/rust-first-logic.md`）。
- R3: 現 `ReviewCommentStore` の file-backed event store が `adaptor/gateway/comment/` に置かれ、usecase が定義した storage port を実装すること。lock・JSON serialization・atomic replace・破損ファイル・欠損ファイル処理を含むこと。
- R4: 8 つの Tauri command wrapper が `adaptor/controller/command/comment/` に置かれ、request / response mapping のみを担い、business behavior を持たないこと。
- R5: `review_comments/watcher.rs` の file watching が、gateway に自然に収まらない場合 `infrastructure` 配下へ置かれること。
- R6: CLI review command（`releash review ...`）が `crate::review_comments` へ直接依存せず、新 usecase 境界経由で動作すること。
- R7: 移行後、`src-tauri/src/review_comments/` が削除され、`lib.rs` に `mod review_comments` が残らず、`adaptor/controller/command/mod.rs` が `crate::review_comments::*` を直接登録しないこと。
- R8: domain / usecase test が正常系・エラー系の list / get / create / append / resolve / delete / history / handoff をカバーし、gateway test が永続化・lock・破損 / 欠損ファイルをカバーすること。
- R9: 外部から観測可能な振る舞い（CLI 出力契約、Tauri command の I/O、永続化フォーマット）が移行前後で変わらないこと（既存テストが通ること）。
- R10: `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通ること。

## 受け入れ基準の概要

- comment の domain ロジックが `domain/comment/` にあり infrastructure 非依存である（R1）。
- application flow が `usecase/comment/` にあり、副作用は port 経由である（R2）。
- file-backed event store が `adaptor/gateway/comment/` にあり storage port を実装する（R3）。
- Tauri command wrapper が `adaptor/controller/command/comment/` にあり mapping に閉じる（R4）。
- watcher が gateway に収まらない場合 `infrastructure` 配下にある（R5）。
- CLI review command が `crate::review_comments` へ直接依存しない（R6）。
- `src-tauri/src/review_comments/` が削除され、`lib.rs` / command 登録に `review_comments` 依存が残らない（R7）。
- domain / usecase / gateway のテストが規定の系をカバーする（R8）。
- 外部観測可能な振る舞いが移行前後で変わらない（R9）。
- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る（R10）。

詳細な受け入れシナリオ（Gherkin）は `behavior.md` で定義する。

## 仮定

- A1: spec ディレクトリ名は `docs/specs/issues-1132` とする（直近 Issue の命名規約 `issues-NNN` に合わせる）。
- A2: 移行は配置移動であり、public 型・Tauri command 名・CLI 出力・永続化フォーマット（state file レイアウト・event JSON）を変更しない。型のリネームや表現の刷新が必要になった場合も本 Issue では行わず、必要なら別 Issue 化する（外部観測可能な振る舞いを変えない方針: [[feedback_behavior_definition_granularity]]）。
- A3: 各層の配置先は Issue #1132 の「責務範囲」に従い、`domain/comment/`・`usecase/comment/`・`adaptor/gateway/comment/`・`adaptor/controller/command/comment/`・（watcher は必要なら）`infrastructure` とする。サブモジュール内の細分（ファイル分割粒度）は design.md で確定する。
- A4: usecase が定義する port（storage、time / id 生成、watcher / notification）の具体的な trait 構成・命名は design.md で確定する。requirements では「副作用が port 経由であること」を要求に留める。
- A5: 既存テスト（`review_comments` module 内 `#[cfg(test)]`）は、対応する層へ移設しつつ、R9（非退行）の回帰検証としても用いる。テストの期待値は実装に合わせて変更しない（仕様が正、実装が誤りなら実装を直す）。
- A6: `cli/mod.rs` への変更は review command を新境界へ繋ぐのに必要な最小限に留め、CLI 全体分割（#1134）には踏み込まない。

## Open Questions

なし。
