# Requirements

対象 Issue: #1133「[impl] root glue / composition root cleanup」

マイルストーン: [12] クリーンアーキテクチャ移行

関連: #1131（WebSocket / sync bridge glue）
Blocks: #878（final sweep）

## Type

リファクタリング（実装 ISSUE）。app behavior / UI 表示を変えずに、crate root 直下に残る platform / app glue を適切な architecture layer へ移し、`lib.rs` を composition root として薄くする。

## 背景と目的

### 背景

- バックエンドは clean architecture へ段階移行中であり、ロジックは layer（`domain` / `usecase` / `adaptor` / `infrastructure`）へ寄せる方針になっている。
- 先行する migration ISSUE の対象に属さない root file が `src-tauri/src/` 直下に残っており、platform integration・file watching・permission mode・test helper などが crate root に散在している。
  - 現状確認（リポジトリ事実）: `src-tauri/src/` 直下に `agent_message_dispatcher.rs` / `app_data_dir.rs` / `cli_install.rs` / `focus_tracker.rs` / `menu.rs` / `native_drop.rs` / `path_aliases.rs` / `permission.rs` / `tray.rs` / `watcher.rs` などが存在し、`lib.rs` の `mod` 宣言（1〜26 行付近）で root module として宣言されている。
  - `src-tauri/src/lib.rs` は約 687 行あり、`run()`（116 行〜）の `setup` クロージャ内に startup orphan cleanup（`record_startup_orphan_cleanup` / `spawn_startup_orphan_cleanup`）、notification listener の business flow（`register_state_change_listener` 内で `usecase::notification::usecase::on_agent_status_changed` を呼ぶ処理）、watcher の spawn、command registration（`register_all` / `register_review_blob_protocol`）などの実装詳細が埋め込まれている。
- 既に `adaptor/controller/command/<context>/`（agent_session / app_config / code / external_editor / hooks / notification / pty_session / repository / telemetry / workflow / workspace_state など）や `infrastructure/`（agent_session / git / pty_session / telemetry）という layer 構造が存在し、移送先の枠組みは整っている。

### 改善する状態

本要求は以下を解消することを目的とする。

1. **crate root の責務肥大**: platform integration や file watching、permission mode が crate root 直下にあり、layer 境界が曖昧。
2. **composition root の肥大化**: `lib.rs` が app construction / DI wiring を超えて、startup cleanup 実装詳細・notification listener の business flow・command registration 詳細を直接保持している。
3. **test-only helper の root 露出**: test 専用 git helper が production module と同じ root 階層に置かれている。

### 目的

- 対象 root file を適切な layer（`adaptor` / `infrastructure` / `domain` / `usecase` / `test_support`）へ移す、または root に残す理由を明記する。
- `lib.rs` を app construction・DI wiring・plugin setup・module registration に限定し、startup task は named な infrastructure / adaptor function へ委譲する。
- Tauri command registration を domain / platform ごとの `register` 関数に寄せる。

## スコープ

先行 migration ISSUE に属さない root file の layer 移送と、`lib.rs` の composition root 化に限定する。

### 対象コード

- `src-tauri/src/agent_message_dispatcher.rs`
- `src-tauri/src/app_data_dir.rs`
- `src-tauri/src/cli_install.rs`
- `src-tauri/src/focus_tracker.rs`
- `src-tauri/src/git/mod.rs` の test helper
- `src-tauri/src/menu.rs`
- `src-tauri/src/native_drop.rs`
- `src-tauri/src/path_aliases.rs`
- `src-tauri/src/permission.rs`
- `src-tauri/src/tray.rs`
- `src-tauri/src/watcher.rs`
- `src-tauri/src/other/utils.rs`（特定 usecase / domain / test-support に寄せるべき helper があれば対象）
- `src-tauri/src/lib.rs` に埋め込まれている startup / wiring behavior

### 責務範囲（移送方針）

- Tauri command は `adaptor/controller/command/<context>/` へ移す。
- platform integration（menu / tray / native drop / focus tracker / cli install / app data dir 等）は `infrastructure/platform/` など明確な infrastructure module へ移す。
- file watching（`watcher.rs`）は repository / code / comment usecase または infrastructure port の背後に置く。
- permission mode（`permission.rs`）は agent / session の domain または usecase 境界に置き、crate root 直下から外す。
- test-only git helper は `test_support` または module-local test support へ移す。
- `lib.rs` は app construction・DI wiring・plugin setup・module registration に限定する。
- startup task（orphan cleanup・notification listener・watcher spawn 等）は named な infrastructure / adaptor function へ委譲する。

## 非スコープ

- app behavior や UI-visible な menu / tray / native-drop behavior の変更（挙動・見た目は変えない）。
- `agent_status_events.rs` の移送（#1131 の対象）。
- WebSocket server / bridge migration（`ws_server/` / `ws_bridge.rs`）: #1131。
- `review_comments/`: #1132。
- `git_host/`: #985。
- `notion/`: #986。
- `protocol/`: #1130。
- `cli/mod.rs` の分割: #1134。
- 全体 usage audit が必要な dead code 削除: #878。

## 要求事項

### R1. root file の layer 移送

- 対象 root file が適切な architecture layer に移動している、または root に残す理由が明記されていること。
- 移送先は責務範囲の方針に従う（command → adaptor、platform integration → infrastructure、file watching → usecase / infrastructure port、permission mode → domain / usecase、test helper → test_support）。

### R2. `lib.rs` の composition root 化

- `lib.rs` が startup cleanup の実装詳細、notification listener の business flow、command registration の詳細を直接保持していないこと。
- startup task は named な infrastructure / adaptor function に委譲されていること。
- `lib.rs` の責務は app construction・DI wiring・plugin setup・module registration に限定されていること。

### R3. root module 宣言の整理

- 旧 glue module の root `mod ...` 宣言が消えている、または composition / test-support として明示的に許可されていること。

### R4. command registration の集約

- Tauri command registration が domain / platform ごとの `register` 関数に寄っていること。

### R5. behavior 不変

- app behavior および UI-visible な menu / tray / native-drop behavior を変更しないこと（純粋な構造変更に留める）。

### R6. 品質ゲート

- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通ること。

## 受け入れ基準の概要

Issue 記載の完了条件に対応する。

- **AC1**: 対象 root file が適切な architecture layer に移動している、または root に残す理由が明記されている（R1）。
- **AC2**: `lib.rs` が startup cleanup 実装詳細・notification listener business flow・command registration 詳細を持っていない（R2）。
- **AC3**: 旧 glue module の root `mod ...` 宣言が消えている、または composition / test-support として明示的に許可されている（R3）。
- **AC4**: Tauri command registration が domain / platform ごとの `register` 関数に寄っている（R4）。
- **AC5**: `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る（R5 / R6）。

## 仮定

- **A1**: spec ディレクトリ名は命名規約に合わせ `docs/specs/issues-1133` とする。
- **A2**: 本 ISSUE は実装 ISSUE であり、設計 doc だけでなく実コードの移送・整理まで含む。
- **A3**: platform integration の移送先となる `infrastructure/platform/` は現状未作成であり、新規 module を作成して集約する。具体的な module 分割粒度（platform 配下のサブ module 構成）は design で確定する。
- **A4**: `permission.rs` の移送先（domain か usecase か）、`watcher.rs` の移送先（usecase か infrastructure port か）といった layer 選択は、責務範囲の方針に沿いつつ design で確定する。
- **A5**: `other/utils.rs` は helper 単位で移送可否を判断し、特定の寄せ先が無い汎用 helper は無理に移送せず、根拠を明記して残す選択も許容する。
- **A6**: behavior 不変の検証は既存テスト（`cargo test`）と、構造変更前後で UI-visible behavior が変わらないことの確認に依拠する。新規の網羅的 UI テスト追加は本 ISSUE のスコープ外とする。

## Open Questions

なし。
