# Design

対象 Issue: #1133「[impl] root glue / composition root cleanup」

マイルストーン: [12] クリーンアーキテクチャ移行

本書は `requirements.md` / `behavior.md` を前提に、crate root 直下に残る platform / app glue の layer 移送と、`lib.rs` の composition root 化の実装方針を確定する。本 ISSUE は behavior 不変のリファクタリングであり、本書のすべての変更は「外部観測可能な app / UI / command 振る舞いを変えない構造変更」に閉じる。

## 概要

`src-tauri/src/` 直下に残る root glue を、責務に応じて `infrastructure` / `adaptor` / `domain` / `test_support` の各 layer へ移送する。あわせて `lib.rs`（現状 687 行）に埋め込まれた startup task（orphan cleanup・notification listener・agent status listener）の実装詳細を named function へ委譲し、`lib.rs` を app construction・DI wiring・plugin setup・module registration に限定する。Tauri command registration は inline の巨大 `generate_handler!` から domain / platform ごとの `register` 関数へ寄せる。

移送はファイル位置と module path の変更が中心であり、関数本体のロジックは原則そのまま移す。公開シグネチャと外部効果（command 集合・起動効果・watch イベント・notification 発火）は保存する。ただし security hardening として `get_review_text_diff` / `get_review_image_diff` は command 集合から削除し、完了通知は Done のみを論理的完了として扱い Closed / Archived では発火しない。

## 変更対象

### 移送対象 root file（現状の事実）

| file | 行数 | 主な内容 | 現在の移送先候補（責務） |
|---|---|---|---|
| `agent_message_dispatcher.rs` | 95 | `dispatch_agent_message`（runtime gateway 経由の message dispatch） | adaptor（concrete infra 依存のため） |
| `app_data_dir.rs` | 15 | `resolve_data_dir` / `TestDataDir` | infrastructure/platform |
| `cli_install.rs` | 330 | `ensure_cli_symlink_installed`（CLI symlink セットアップ） | infrastructure/platform |
| `focus_tracker.rs` | 77 | `FocusTracker`（window focus 状態） | infrastructure/platform |
| `menu.rs` | 200 | `setup_menu` + `set_menu_items_enabled`(command) + `MenuItemsState` + `ids` | infrastructure/platform（setup）＋ adaptor/command（command） |
| `native_drop.rs` | 202 | `install`（macOS native drop） | infrastructure/platform |
| `path_aliases.rs` | 686 | `PathAliases` / `BuildProfile` / `prepare_child_env` / env 解決 | infrastructure/platform |
| `permission.rs` | 158 | `PermissionMode`（value object） | domain/agent_session/value_objects |
| `tray.rs` | 79 | `setup_tray` + `QUIT_REQUESTED` + `ids` | infrastructure/platform |
| `watcher.rs` | 132 | `FileWatcherManager` + 3 Tauri command | infrastructure（manager）＋ adaptor/command（command） |
| `test_support.rs` | 8 | `build_session_store`（test 専用） | test_support（root に明示許可で残置・dir 化） |
| `other/utils.rs` | 44 | `unix_timestamp_seconds` / `relative_path` | other（横断 util として残置、根拠明記） |
| `git/mod.rs` の `#[cfg(test)]` helper | 79 | `create_test_repo` 等の git fixtures | test_support |

### `lib.rs` から抽出する behavior

- `record_startup_orphan_cleanup` / `spawn_startup_orphan_cleanup`（39〜113 行、orphan cleanup の thread spawn・telemetry 記録）
- notification listener closure（353〜406 行、notify config と inactivity を読み snapshot を作り webhook send へ渡す flow）
- agent status listener closure（435〜455 行、`AgentStatusCenter` と `agent_status_events::emit_agent_status_changes` の wiring）
- command registration の inline 列挙（`register_all` 内 `generate_handler!` 中の watcher / menu 部分）

### 非対象（requirements 非スコープと整合）

`agent_status_events.rs`（#1131）/ `ws_server/` / `ws_bridge.rs`（#1131）/ `review_comments/`（#1132）/ `git_host/`（#985）/ `notion/`（#986）/ `protocol/`（#1130）/ `cli/mod.rs` 分割（#1134）/ 網羅的 dead code 削除（#878）。

## アーキテクチャと責務分割

レイヤー依存方向は `src-tauri/AGENTS.md` に従う:
`infrastructure → adaptor/gateway → domain ← usecase ← adaptor/controller`。

### 1. `infrastructure/platform/`（新設）

platform integration を集約する新規 module。Tauri / OS API へ直接依存するため infrastructure に置く。サブ module 分割は file 単位の凝集を保つ:

```
infrastructure/platform/
├── mod.rs              // pub use の再エクスポートのみ
├── menu.rs             // setup_menu, MenuItemsState, ids（command を除く）
├── tray.rs             // setup_tray, QUIT_REQUESTED, ids
├── native_drop.rs      // install, NativeFileDrop
├── focus_tracker.rs    // FocusTracker
├── cli_install.rs      // ensure_cli_symlink_installed, should_install_cli_symlink_for_profile
├── app_data_dir.rs     // resolve_data_dir, TestDataDir
└── path_aliases.rs     // PathAliases, BuildProfile, prepare_child_env, env 解決
```

- `menu.rs` / `tray.rs` / `native_drop.rs` / `focus_tracker.rs` / `cli_install.rs` / `app_data_dir.rs` / `path_aliases.rs` は本体ロジックをそのまま移送する。
- `menu.rs` の `#[tauri::command] set_menu_items_enabled` のみ adaptor へ分離する（後述 3）。menu setup・state・ids は infrastructure に残す。
- `path_aliases.rs` は CLI（`cli/mod.rs`）と pty backend gateway から参照されるため、move 後も `crate::infrastructure::platform::path_aliases::...` で参照可能にする。pure logic（`BuildProfile` / alias 名決定）の domain への further 分離は本 ISSUE では行わず whole-move に留める（理由はリスクと代替案で後述）。

### 2. `infrastructure/file_watcher/`（新設・watcher の infra 部分）

`watcher.rs` の `FileWatcherManager`（`notify` crate 依存の watcher セッション管理）を infrastructure へ移す。Tauri command は含めない。

```
infrastructure/file_watcher/
└── mod.rs              // FileWatcherManager
```

### 3. `adaptor/controller/command/` への command 集約（R4）

現状 `register_all`（`adaptor/controller/command/mod.rs`）は巨大な inline `generate_handler!`（fallback handler）に watcher / menu 等の command を直接列挙し、新しい context は `<context>::register(&mut router)` で domain route を追加する 2 系統になっている。本 ISSUE では root 由来 command を context の `register` 経路へ寄せる:

- 新規 `adaptor/controller/command/watcher/`（`mod.rs` + `register`）に `start_watching` / `start_git_dir_watching` / `stop_watching` を移し、`FileWatcherManager`（infra）を呼ぶ thin command にする。
- 新規 `adaptor/controller/command/menu/`（`mod.rs` + `register`）に `set_menu_items_enabled` を移し、`infrastructure::platform::menu` を呼ぶ thin command にする。
- `register_all` の inline `generate_handler!` から上記 command を除去し、`watcher::register(&mut router)` / `menu::register(&mut router)` を追加する。

これにより「Tauri command registration が domain / platform ごとの `register` 関数に寄る」（R4 / AC4）を満たす。command 名と invoke 互換性は変えない（behavior.md「command 集合一致」Rule）。

### 4. `domain/agent_session/value_objects/permission_mode.rs`（permission mode）

`PermissionMode` は外部依存を持たない value object（`parse` / `as_str` / `allowed_list` / `Display`）であり、agent session domain の概念。domain 層の外部依存禁止に従い `serde` などの転送・保存形式依存は持たせない。requirements R1 の「permission mode → domain / usecase」に従い `domain/agent_session/value_objects/permission_mode.rs` へ移し、`value_objects/mod.rs` と `domain/agent_session/mod.rs` から他の value object と同じ形で re-export する。JSON / 保存 / Tauri 境界の文字列表現は adaptor / gateway / controller 側で `as_str` / `parse` を使って扱う。参照元（`agent_message_dispatcher` / `other/telemetry/attributes` / `adaptor/controller/command/agent_session/permission`）は use path を更新する。

### 5. `agent_message_dispatcher.rs` の移送（adaptor）

`dispatch_agent_message` は `infrastructure::agent_session::runtime_gateway` の concrete 型を直接 import しており、usecase layer（domain のみ依存）には置けない。唯一の呼び出し元は `adaptor/controller_support.rs`。したがって adaptor layer の controller support 配下へ移す:

- 移送先: `adaptor/controller/agent_session/message_dispatch.rs`（または `controller_support` 隣接 module）。本体ロジックは保存し、`crate::permission::PermissionMode` → `crate::domain::agent_session::PermissionMode`、`crate::app_data_dir::resolve_data_dir` → `crate::infrastructure::platform::app_data_dir::resolve_data_dir` へ use を更新する。

### 6. `test_support`（test 専用 helper の集約）

- 既存 root `test_support.rs`（`#[cfg(test)]`）を `test_support/` dir へ拡張し、`test_support/mod.rs`（`build_session_store`）+ `test_support/git.rs`（git fixtures）に分ける。
- `git/mod.rs` の `#[cfg(test)]` git helper（`create_test_repo` / `create_initial_commit` / `add_and_commit` / `setup_remote_repo`）を `test_support/git.rs` へ移す。参照元（`usecase/repository_state/*` の test）は `crate::test_support::git::...` で参照。
- `test_support` は root module だが `#[cfg(test)]` の test-support として R3 の「composition / test-support として明示的に許可」に該当し、root 残置を許容する（`lib.rs` 上にコメントで意図明記）。

### 7. `other/utils.rs`（横断 util の残置判断）

`other/` は `src-tauri/AGENTS.md` で「エラー型・ログ等の横断的関心事」と定義された正規の置き場。`unix_timestamp_seconds` / `relative_path` は複数 layer から使われる generic helper で、特定 domain / usecase への自然な寄せ先がない。requirements A5 に従い `other/utils.rs` に残置し、残置理由（横断 util・単一 domain 専有でない）を本書に明記する。実装時に単一 domain 専有と判明した helper があれば、その helper のみ当該 domain へ移す。

### 8. `lib.rs` の composition root 化（R2 / AC2）

抽出方針:

- **orphan cleanup**: `record_startup_orphan_cleanup` / `spawn_startup_orphan_cleanup` を `infrastructure/agent_session/startup.rs`（または既存 `runtime` 配下の named module）へ移す。`lib.rs` の `setup` からは `infrastructure::agent_session::startup::spawn_startup_orphan_cleanup(...)` を 1 行呼び出すだけにする。telemetry 記録・panic ハンドリング・thread 名はすべて移送先に保持する。`#[cfg(test)]` の `STARTUP_ORPHAN_CLEANUP_*` カウンタとテストも移送先へ移す。
- **notification listener**: `register_state_change_listener` へ渡す notification closure（config 読込→snapshot 生成→send）を named な wiring 関数 `adaptor::controller::notification_wiring::register_agent_notification_listener(session_store, notification_usecase)` へ抽出する。`lib.rs` は呼び出しのみ。Active / Idle / Error / Done は従来通り通知対象、Closed / Archived は snapshot を生成せず非通知にする。
- **agent status listener**: `AgentStatusCenter` への listener 登録（`emit_agent_status_changes` wiring）を `adaptor::controller::agent_status_wiring::register_agent_status_listener(...)` へ抽出する。
- **application lifecycle**: tray quit から呼ばれる終了処理は `adaptor::controller::application_lifecycle::request_application_quit` へ置く。
- **DI wiring 本体**は composition root の正当な責務として `lib.rs` に残す（usecase / gateway の Arc 組み立てと `app.manage`）。requirements R2 が要求するのは「startup cleanup 実装詳細・notification business flow・command registration 詳細を持たないこと」であり、DI 配線そのものの追い出しは要求していない。

抽出後の `lib.rs run()` は概ね「runtime 構築 → builder 構築 / plugin / manage → setup 内で named wiring 関数群を呼ぶ → register_all → run」の薄い構成になる。

### 9. root `mod` 宣言の整理（R3 / AC3）

`lib.rs` 冒頭（1〜26 行）の root `mod` 宣言から、移送した module（`agent_message_dispatcher` / `app_data_dir` / `cli_install` / `focus_tracker` / `menu` / `native_drop` / `path_aliases` / `permission` / `tray` / `watcher`）を削除する。`test_support` は test-support として残し意図コメントを付す。`agent_status_events`（#1131）はスコープ外のため残置。

## データモデルまたは型

新規型は導入しない。既存型はすべて move のみで定義内容を保存する:

- `PermissionMode`（enum, serde 非依存 value object）— variant・canonical string（`ask` / `edit` / `full`）・parse/display の挙動を保存する。agent message / telemetry / config / 保存ファイルとの互換に必要な JSON・保存表現は adaptor / gateway / controller 境界で文字列として扱う。
- `FileWatcherManager` / `MenuItemsState` / `FocusTracker` / `PathAliases` / `BuildProfile` / `NativeFileDrop` / `TestDataDir` — フィールド・公開 API を保存。
- `QUIT_REQUESTED: AtomicBool` — `lib.rs` の run-event handler（595 行）から参照されるため、`infrastructure::platform::tray::QUIT_REQUESTED` として pub 参照可能にする。

`#[tauri::command]` の関数シグネチャ（引数名・型・戻り値）は frontend `invoke` 互換のため一切変更しない。

## 処理フロー

移送による実行時フローの変更はない。代表フローの move 後経路:

1. **起動 (lib.rs run → setup)**: plugin/manage → `path_aliases::ensure_release_data_dir_env_for_app` → `cli_install::ensure_cli_symlink_installed`（いずれも `infrastructure::platform` 経由）→ config 読込・DI 配線 → `notification_wiring::register_agent_notification_listener` / `agent_status_wiring::register_agent_status_listener` → watcher spawn → `menu::setup_menu` / `tray::setup_tray`（quit は `application_lifecycle::request_application_quit`）/ `native_drop::install`（`infrastructure::platform` 経由）→ `spawn_startup_orphan_cleanup`（infrastructure）。
2. **command invoke**: frontend `invoke("start_watching", ...)` → `register_all` の router → `watcher::register` で登録された handler → `infrastructure::file_watcher::FileWatcherManager`。menu command も同様に `menu::register` 経由。
3. **agent status 変化 → notification**: `SessionStore` state 変更 → `adaptor::controller::notification_wiring` で登録された listener closure → `AgentSessionNotificationUsecase` の snapshot / send。Active / Idle / Error / Done は通知対象、Closed / Archived は `None` として snapshot / send を行わない。

## エラー処理

- 移送に伴う新規エラー型は追加しない。各関数の `Result` / `?` 伝播・`log::warn!` 等のログは現状を保存する。
- orphan cleanup の panic catch（`catch_unwind`）・spawn 失敗時の telemetry 記録・gate open は移送先へそのまま移す（起動時外部効果不変、behavior.md「orphan cleanup の外部効果一致」）。
- module ごとの専用 error type 方針（AGENTS.md）に新たに反する変更はしない。

## テスト方針

- **既存テストの保全**: 移送した module 内の `#[cfg(test)]`（`path_aliases` の env 解決テスト、`cli_install` の profile 判定テスト、`permission` の parse テスト、orphan cleanup テスト等）は move 先に同梱し、期待値を変更しない（behavior.md「テスト期待値の書き換えをしない」）。
- **test fixtures**: `git/mod.rs` の test helper・`test_support` を `test_support/` へ集約後、参照する test が緑のままであることを確認する。
- **command 集合の期待値**: `register_all` 変更後に各 domain の `COMMAND_NAMES` 全件が当該 domain に route されることを担保する。さらに全 domain の `COMMAND_NAMES` 結合集合を canonical 期待集合と比較する。canonical 期待集合は security hardening で削除する `get_review_text_diff` / `get_review_image_diff` を含まない削除後集合とする。
- **品質ゲート（R6 / AC5）**: `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` をすべて通す。frontend 側は move の影響を受けないが、CI 同等の `pnpm lint` / `pnpm test` も最終確認する。
- **behavior 不変検証**: 自動テストに加え、起動後の menu / tray / native drop / focus 追従 の UI-visible 動作が変わらないことを手動確認（behavior.md の各 Scenario に対応）。

## リスクと代替案

- **module path 変更による広範な use 更新**: `app_data_dir`（25+ 箇所）・`path_aliases`（CLI 含む）・`permission_mode`（複数 layer）は参照点が多く、use path 更新漏れが compile エラーになる。対策: move ごとに `cargo build` / `clippy` で網羅検出する（漏れは behavior でなく compile で顕在化するため安全）。
- **command registration 変更による command 欠落**: `register_all` から inline 列挙を外す際に command を落とすと「command 集合一致」Rule を破る。対策: 各 domain の `COMMAND_NAMES` 全件 route と canonical 期待集合の突き合わせで検出する。canonical 期待集合は `get_review_text_diff` / `get_review_image_diff` 除外後の集合で固定する。
- **`path_aliases` を whole-move に留める判断**: pure logic（`BuildProfile` 等）を domain へ分離する案もあるが、686 行・CLI 共有・env 解決の凝集が高く、分離は参照点を増やしリスクが上がる。本 ISSUE は behavior 不変が主目的のため whole-move（infrastructure/platform）に留め、domain 分離は別 ISSUE の余地として残す。
- **`menu` / `watcher` の command と本体の分離**: 1 file を「infra 本体 + adaptor command」に割ると凝集が一時的に下がるが、layer 境界（command は adaptor、OS 連携は infrastructure）の明確化と R4 達成のため分離を採る。代替（command を infrastructure に残す）は R4 を満たせず却下。
- **`agent_message_dispatcher` の adaptor 配置**: usecase が望ましいが concrete infra 依存のため不可。将来 runtime gateway を port 抽象化すれば usecase へ移せるが、それは本 ISSUE スコープ外（gateway 抽象化を伴うため）。

## 仮定

- **D1**: requirements A3 に従い `infrastructure/platform/` を新設し、platform glue（menu / tray / native_drop / focus_tracker / cli_install / app_data_dir / path_aliases）を file 単位サブ module で集約する。
- **D2**: requirements A4 の layer 選択を本書で確定する — `permission.rs` → `domain/agent_session/value_objects/permission_mode.rs`、`watcher.rs` の manager → `infrastructure/file_watcher`・command → `adaptor/controller/command/watcher`。
- **D3**: `agent_message_dispatcher.rs` は concrete infra 依存のため adaptor（controller support 配下）へ置く。usecase 移送は port 抽象化を要するためスコープ外。
- **D4**: requirements A5 に従い `other/utils.rs` の generic helper は `other/` に残置し、残置理由を本書に明記する。
- **D5**: `test_support` は `#[cfg(test)]` の test-support として root 残置を許容し（R3）、git fixtures を集約する dir へ拡張する。
- **D6**: 移送は定義内容・公開シグネチャ・外部効果を保存する pure structural move とし、ロジック改変・dead code 削除は行わない（#878 スコープ）。
- **D7**: `path_aliases.rs` は whole-move に留め、pure logic の domain 分離は行わない。

## Open Questions

なし。
