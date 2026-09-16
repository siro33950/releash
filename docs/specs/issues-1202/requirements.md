# Context

- 要求の正本: Issue #1202「A-daemon: Tauri 非依存の headless デーモン抽出」、milestone 77「01. ローカル server-client 化（基盤）」。
- 背景資料: Issue #1199（A1）、Issue #1200（A2）、Issue #1201（A-flip）、Issue #1203（A-launchd）、PR #1824、`docs/specs/issues-1199/requirements.md`、`docs/specs/issues-1200/requirements.md`、`docs/specs/issues-1201/requirements.md`、`AGENTS.md`。
- milestone 77 で確定済みの設計判断のうち、本変更が従うもの。
  - 判断①: req/resp は Protocol Buffers で定義したエンベロープ（`request_id`＋`oneof command`）を経由して usecase 共有 dispatch へ渡す。push も proto message とする。`.proto` が protocol の正であり、Rust / client の型はそこから生成する。
  - 判断③: クライアント通信は WebSocket 単一トランスポートとする。
  - 判断④: ローカルは loopback＋token ファイル認証とする。
  - 判断⑤: headless デーモン抽出＋launchd LaunchAgent 常駐＋1 .app 同梱。
  - 判断⑥: strangler 移行（A0 掃除 → A1 walking skeleton → ドメイン被覆 → 既定切替 → デーモン抽出 → 常駐）。本変更はデーモン抽出にあたり、既定切替（#1201、PR #1824 で merge 済み）の後、常駐（#1203）の前に行う。
  - 判断⑦: クライアント token は discovery file の master token と分けて発行する。権限が同等でも識別子を分け、master token を renderer へ露出させない。
  - 判断⑧: terminal stream は汎用エンベロープに統合し、クライアント接続は 1 本にする。stream 単位のフロー制御（`attachment_id`＋`sequence` / `ack`）はエンベロープ内に保持する。
- milestone 77 の土台。
  - クライアント向け ws は、MS82 で新設した local API（axum、127.0.0.1 bind、discovery file）の上にある。
  - HTTP local API（workflow / provider-lifecycle）は CLI と provider hook のローカル専用入口として残す。クライアントの経路ではない。
  - A0 で温存した旧 ws shell は #1338 で削除済みであり、再利用しない。
- milestone 77 の成果は、ローカルで desktop が daemon＋ws で完全動作すること（desktop 1 クライアント。CLI / hook の HTTP 入口は対象外）であり、Track B（ネイティブ UI）と Track C（リモート）の土台となる。
- Issue #1202 の「現状」節は #1201 の merge 前の記述である。本文書の Current Behavior は、`feat/issues/1202`（`b05280f9`）のコードで確認した状態を正とする。
- Issue #1202 は、実装上の次の指定を含む。これらは Design で扱う。
  - `AppHandle` 引数を明示依存に置き換え、managed state を単一の composition root に集約する。
  - `infrastructure/platform/` 配下（menu / tray / window_lifecycle / native_drop / app_data_dir）は desktop shell 側に残す。
  - バイナリ構成（現行 1 バイナリの GUI / CLI 分岐に daemon モードを足すか、分割するか。`/usr/local/bin/releash` symlink と `releash hook` の受け口を含む）は本 Issue の設計で決める。
- Issue #1203 は次を定める。
  - .app に UI と daemon を同梱し、UI が daemon を起動・監視・停止する（UI 起動時の spawn と ws 接続完了までの待機、異常終了時の再 spawn、親の終了検知による daemon の自己終了、Quit 時の停止要求と一括停止完了の待機）。
  - daemon の起動失敗と再起動制御、更新・再起動時の UI と daemon の切替。
  - CLI に daemon 単体を起動する入口を作らない（daemon 未起動時の CLI の挙動は現状のまま）。
- `docs/specs/issues-1201/requirements.md` は、「接続先の backend の再起動」で変更要求の結果を確定できない場合の扱い（#1201 R-011）について、backend のプロセスを実際に再起動した状態での確認はデーモン抽出（#1202）と daemon の起動監督（#1203）で扱うと定めている。

# Outcome

- 対象者は、desktop 利用者と、常駐（#1203）・ネイティブ UI（Track B）・リモート（Track C）を実装する開発者である。
- 現在、desktop と backend の通信はクライアント ws に統一されているが、backend は Tauri アプリと同じプロセスの中で Tauri の起動処理に組み込まれて動作し、data_dir の解決、状態の取得、外部エディタの起動、プロセスの終了を Tauri に依存している。このため backend を desktop なしで起動できず、desktop 以外のクライアントの接続先や、UI と独立した常駐の土台にならない。
- 変更後は、backend が Tauri に依存しない headless の daemon として desktop とは別のプロセスで動作し、単独で起動してもクライアント ws で接続・command 実行ができる。desktop は起動済みの外部の daemon へクライアント ws で接続する UI shell として、変更前と同じ機能を提供する。desktop による daemon の起動・監視・停止・再起動は #1203 で扱う。daemon は変更前と同じ data_dir を使い、CLI と provider hook も変更前と同じく動作する。

# Current Behavior

最初の周の開始時点（`feat/issues/1202`、`b05280f9`）で、コードを読んで確認した挙動。アプリケーションの起動・テストの実行による確認は行っていない。

## Issue 本文の現状記述との差

- Issue #1202 は `app.emit` 直呼びが残るとしているが、backend 状態の push 8 種（`agent-session-changed` / `branch-list-sync` / `file-change` / `git-status-changed` / `repo-paths-changed` / `repository-snapshot-changed` / `review-comments-changed` / `workflow-execution-changed`）は、`src-tauri/src/adaptor/gateway/push.rs` の `BackendPush::emit` がクライアント ws の `PushSink` へ proto の Envelope として送るだけであり、Tauri event では配信していない。`BackendPush::emit` は `PushSink` を `AppHandle` の managed state から取得する。
- `src-tauri/src/infrastructure/comment/watcher.rs` の `app.emit("review-comments-changed", ...)` は `#[cfg(test)]` のテストモジュール内だけにある。
- 本番コードに残る Tauri emit は、UI shell の事象 `menu-event`（`infrastructure/platform/menu.rs`）と `native-file-drop`（`infrastructure/platform/native_drop.rs`）だけである。
- Issue #1202 が desktop shell 側に残すとする `command/code/review_blob.rs` は PR #1824 で削除済みであり、`review-blob:` URI scheme は存在しない。review の画像はクライアント ws で取得する。
- Issue #1202 は結合箇所を「command ディレクトリ外の 24 ファイル」「`app.manage` 約 30 箇所」としている。現行では `adaptor/controller/command/` 外で `AppHandle` を含むファイルは 30 件、`lib.rs` の起動処理（`run`）内の `app.manage` は 36 箇所である。

## プロセスと起動

- 実行ファイルは `releash` の 1 つである（Cargo package `releash`、lib `releash_lib`）。`src-tauri/src/main.rs` は、引数がない場合と `--hidden` だけの場合に GUI（`releash_lib::run`）を起動し、それ以外は CLI（`cli::run`、`workflow` / `review` / `hook`）として実行する。
- backend は Tauri アプリと同じプロセスで動作し、独立した daemon は存在しない。backend の組み立ては `lib.rs` の Tauri `setup` 内で行われる。
- 起動時、data_dir を `app.path().app_data_dir()` で解決し、local event store を開く。解決または store を開くことに失敗すると、起動失敗用のウィンドウを作り、`get_application_startup_outcome` と `quit_after_startup_failure` を Tauri invoke で受け付ける。この場合 local API は bind されない。
- 起動に成功すると、local API を 127.0.0.1 の空きポートに bind し、`<data_dir>/local-api.json`（port、master token、instance_id、pid、process_started_at）を書き出す。renderer 用のクライアント token は master token とは別に生成し、discovery file には書き出さない。その後 main window を作り、ファイルドロップの受け口とウィンドウの初期表示状態（`--hidden`・設定）を適用する。
- 起動時に、release build（debug build と performance build を除く）では `/usr/local/bin/releash` を実行中の実行ファイルへの symlink として設置する（`infrastructure/platform/cli_install.rs`）。

## data_dir の解決

- desktop の data_dir は Tauri の `app_data_dir()` で解決され、`<OS のデータディレクトリ>/<bundle identifier>` になる。identifier は `src-tauri/tauri.conf.json` の `com.releash.app`、`pnpm tauri:dev` が使う `src-tauri/tauri.conf.dev.json` の `com.releash.app.dev`、performance build が使う `src-tauri/tauri.conf.performance.json` の `com.releash.app.performance` である。desktop は自身の data_dir の解決に `RELEASH_DATA_DIR` を使わない。
- CLI の data_dir は、`RELEASH_DATA_DIR` が空でなければその値、そうでなければ `dirs::data_dir()` にビルド種別の既定名（release: `com.releash.app`、debug: `com.releash.app.dev`）を連結した場所になる（`src-tauri/src/cli/common.rs`、`infrastructure/platform/path_aliases.rs`）。
- `resolve_data_dir(app)`（`infrastructure/platform/app_data_dir.rs`）は、terminal の checkpoint・shell integration・子プロセス環境（`adaptor/gateway/terminal_surface/runtime_gateway_impl.rs`）、workflow host（`adaptor/gateway/workflow/workflow_host.rs`）などから `AppHandle` を介して呼ばれる。

## 子プロセスからの CLI

- terminal の子プロセスには、`<data_dir>/bin/<alias>`（release: `releash`、debug: `releash-dev`）に実行中の実行ファイルを呼ぶ wrapper を置いて PATH の先頭に加え、`RELEASH_DATA_DIR` を設定する（`path_aliases::prepare_child_env`）。
- 起動時に、親プロセスから別の Releash 由来の `RELEASH_DATA_DIR` を継承している場合は、自身の data_dir へ置き換える（`path_aliases::ensure_release_data_dir_env_for_resolved_path`）。

## desktop の Tauri IPC

- renderer の Tauri invoke は `get_client_endpoint`（`src/lib/clientSocket.ts`。クライアント ws の URL `ws://127.0.0.1:<port>/v1/client` と認証 subprotocol）、`get_application_startup_outcome`・`quit_after_startup_failure`・`set_menu_items_enabled`（`src/App.tsx`）だけである。backend が Tauri invoke handler に登録する command もこの 4 つだけである。
- renderer の Tauri event 購読は `menu-event` と `native-file-drop` だけである。
- renderer は dialog、opener、updater＋process relaunch、autostart を Tauri plugin で使う。

## backend の Tauri 依存

- `adaptor/gateway/workflow/workflow_host` とその submodule（approval_runtime / command_preparation / delegate / isolated_worktree / lifecycle_commands / runtime_session）、`runtime_command_gateway.rs`、`terminal_surface/runtime_gateway_impl.rs`、`workflow/secret_source.rs`、`workflow/event_log_writer.rs` などは `AppHandle` を受け取り、managed state や data_dir を取得する。
- 外部エディタの起動（`adaptor/gateway/external_editor/launcher_impl.rs`）は `tauri_plugin_opener` を使う。
- application quit（menu・tray の Quit と、クライアント ws の `request_application_quit`）は、`ShutdownCoordinator` が workflow runtime / terminal surface / provider exit observer / local API を一括停止した後、意図に応じて Tauri の `exit` または `request_restart` で同じプロセスを終了または再起動する（`adaptor/controller/application_lifecycle.rs`）。
- `infrastructure/platform/` には app_data_dir / menu / native_drop / tray / window_lifecycle（Tauri を使う）と、cli_install / file_replace / path_aliases（Tauri を使わない）がある。file_replace は review comment の保存と local event store の保守から使われる。

# Scope / Non-goals

## 変更するもの

- backend を、Tauri に依存せず desktop とは別のプロセスで動作する headless の daemon にすること。
- desktop を、外部の daemon へクライアント ws で接続する UI shell にすること。
- daemon の data_dir の解決を Tauri から切り離し、変更前と同じ場所を指すようにすること。
- 実行ファイルの構成の変更に伴う、`/usr/local/bin/releash`、子プロセスの CLI alias、`releash hook` の受け口の維持。

## 変更しないもの

- desktop の起動時の daemon の起動（spawn）と接続完了までの待機、daemon の監視、異常終了時の再 spawn、親の終了検知による daemon の自己終了、daemon の起動失敗と再起動制御、daemon の起動失敗時の desktop での失敗理由の表示と終了操作、メニュー・トレイの Quit での daemon への停止要求と一括停止完了後の desktop の終了、application quit の再起動（Restart）の意図での daemon と desktop の再起動、launchd LaunchAgent による常駐、ログイン時に起動・最小化状態で起動の設定、1 .app への同梱、更新・再起動時の UI と daemon の切替（#1203）。
- UI shell に残す物（dialog（フォルダ選択）、opener、updater＋process relaunch、autostart、`native-file-drop`、`menu-event`、menu / tray / window lifecycle）をクライアント ws に載せること。
- CLI と provider hook が使う HTTP local API（workflow / provider-lifecycle）の入口。
- `.proto` を protocol の正とするエンベロープの形式（判断①）。
- #1201 が定めた、変更要求の未送信・結果不明・結果確定の区別、期限、生存確認、再接続の規則。
- リモートアクセス経路と、複数クライアント。
- `review-blob:` URI scheme（PR #1824 で撤去済み）。

# Requirements

- R-001: daemon は desktop とは別のプロセスとして動作し、ウィンドウ・メニュー・トレイアイコンを表示しない。daemon の実行ファイルは Tauri に依存しない。
- R-002: daemon を desktop なしで単独起動すると、daemon は data_dir に local API の discovery file を書き出し、クライアント token で認証したクライアント ws 接続を受け付け、エンベロープで送られた command を実行して、同じ `request_id` の応答を返す。
- R-003: desktop は別のプロセスで動作する daemon へクライアント ws で接続し、変更前の desktop で利用できた機能（UI shell に残す物を含む）が、変更前と同じ結果で動作する。ただし、Non-goals で #1203 の範囲とした、daemon の起動失敗時の表示と終了操作、メニュー・トレイの Quit、application quit の再起動（Restart）の意図は除く。
- R-004: desktop と daemon の間の req/resp、backend 状態の push、terminal の入出力は、クライアント ws だけで行われる。backend 状態の push は Tauri event で配信されない。
- R-005: daemon が使う data_dir は、ビルド種別ごとに変更前の desktop が使っていた場所（release: `<OS のデータディレクトリ>/com.releash.app`、dev: `<OS のデータディレクトリ>/com.releash.app.dev`、performance: `<OS のデータディレクトリ>/com.releash.app.performance`）であり、変更前に保存されたデータを変更前と同じく読み書きできる。
- R-006: renderer には discovery file の master token が渡されない。renderer がクライアント ws の認証に使う token は master token と異なる。
- R-007: daemon の local API とクライアント ws は、127.0.0.1 でだけ接続を受け付ける。
- R-008: CLI（`releash workflow` / `releash review`）と provider hook（`releash hook`）は、変更前と同じ data_dir の解決で daemon の local API または data_dir に到達し、変更前と同じ結果を返す。ただし、performance build で `RELEASH_DATA_DIR` が未設定または空のときの既定の data_dir は、daemon と同じ `<OS のデータディレクトリ>/com.releash.app.performance` とする。
- R-009: release build の Releash の起動時に、変更前と同じ条件で `/usr/local/bin/releash` が設置され、`/usr/local/bin/releash` から Releash の CLI を実行できる。
- R-010: daemon が起動する terminal と agent session の子プロセスから、変更前と同じ alias（release: `releash`、dev: `releash-dev`）で Releash の CLI を実行でき、その CLI は daemon と同じ data_dir を使う。
- R-011: desktop の接続先の daemon が停止し、別のインスタンスとして起動し直された場合、desktop は新しいインスタンスへ再接続し、現在の状態が画面へ反映される。結果を確定できない変更要求は結果不明と表示され、安全を確認できない再実行は行われない。
- R-012: desktop は daemon を起動しない。desktop は、同じビルド種別の data_dir にある discovery file を手がかりに、起動済みの daemon へクライアント ws で接続する。
- R-013: daemon を desktop なしで単独起動する手段は、protocol smoke test と開発のためのものであり、利用者向けの入口として提供されない。Releash の CLI のヘルプと利用者向けガイド（`docs/guide/`）に、daemon を単独起動する手段は現れない。
- R-015: daemon は application quit の終了（Exit）または再起動（Restart）の意図を受けると、変更前と同じ一括停止（workflow runtime / terminal surface / provider exit observer / local API）を行い、その完了後に daemon のプロセスが終了する。

# Assumptions / Open Questions

なし。
