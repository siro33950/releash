# Design

本書は `requirements.md` / `behavior.md` を実装方針へ落とし込む設計文書である。本 Issue は土台リファクタ（決定済みの削除掃除）であり、機能追加・挙動変更は行わない（リモートアクセス起動トリガーの除去のみ意図された削除）。

## 概要

A1（walking skeleton）が乗る土台を整えるため、以下を削除/縮退する。

1. ドリフトしたリモートクライアント `src/remote/` 一式と remote ビルド経路（`vite.config.remote.ts`・`build:remote`・remote 専用依存）。
2. リモートアクセス起動トリガー一式（フロントの起動 UI・Tauri command 表層・tray・auto-start 配線・remote 起動専用 config）。
3. ドリフトした ws の req/resp ハンドラ群（約 26）と、それ専用の `WsMessage` variant（`*Request`/`*Response` 等）・ルーティング分岐・ハンドラ実装・バリデーション・テスト。

縮退後、ws は「**認証 + push broadcast の shell**」になる。`http.rs`/`auth.rs`/`rate_limit.rs`/`WsBroadcaster`（案X）と `start_server_core`/`stop_server_core` は A1 再利用のため残置し、呼び出し元ゼロになる分は `#[allow(dead_code)]` で温存する。デスクトップが `invoke`/`emit`/`listen` で利用する下層ロジック（usecase/gateway/domain）は保持する。

## 変更対象

### フロントエンド削除（`src/`）

- `src/remote/` ディレクトリ一式（`RemoteApp` とその配下の `components/`・`hooks/`・`styles/`・`main.tsx` 等。テスト 12 ファイルを含む全ファイル）。
- `src/components/panels/RemotePanel.tsx` ＋ `RemotePanel.test.tsx`。
- `src/hooks/useRemoteServer.ts` ＋ `useRemoteServer.test.ts`。
- `src/hooks/useRemoteAutoStart.ts` ＋ `useRemoteAutoStart.test.ts`。
- `src/hooks/useRemoteConfig.ts`（remote 起動専用 config の取得/保存フック。`SettingsModal` の "Remote" タブ削除に伴い不要化）。
- `src/components/panels/SettingsModal.tsx` の "Remote" 設定タブ：タブ定義（`{ id: "remote", ... }`）、`RemoteSection` コンポーネント（auto-start トグル群）、`useRemoteConfig` の import/利用、`remoteIsDirty`/`remoteSave` を絡める保存配線、`Globe` アイコン import 等の参照一切。`SettingsModal.test.tsx` から remote タブ関連アサーションを除去。

### フロントエンド参照除去（`src/App.tsx`）

- `import { RemotePanel }`（4 行目相当）、`import { useRemoteAutoStart }`（16 行目相当）。
- `useRemoteAutoStart(!initializing)` 呼び出し（34 行目相当）。
- `showRemote` 状態と RemotePanel を表示する Dialog（185–193 行目相当）。RemotePanel を開く UI トリガー（ボタン等）も併せて除去。

### ビルド設定削除（リポジトリルート）

- `vite.config.remote.ts` を削除。
- `package.json`:
  - スクリプト `build:remote` を削除。
  - `build` スクリプトから `pnpm build:remote` 参照を除去。
  - `dev` スクリプトから `vite build --watch --config vite.config.remote.ts` 並行実行を除去。
  - remote 専用依存 `html5-qrcode`・`@noble/hashes` を削除（**仮定 A**: 後述の調査で `src/remote/` 以外に参照なしを確認済み）。
  - `@xterm/xterm`・`@xterm/addon-fit` は**残置**（デスクトップの `src/components/panels/TerminalPanel.tsx`・`src/hooks/useTerminal.ts`・`src/test/setup.ts` でも使用＝remote 専用ではない）。
- `pnpm-lock.yaml` は依存削除に合わせて再生成（`pnpm install`）。

### CI 削除（`.github/workflows/ci.yml`）

- Rust ジョブの `pnpm build:remote` ステップ（395 行目相当の `pnpm install --frozen-lockfile && pnpm build:remote && pnpm build:bridge` から `pnpm build:remote &&` を除去）。
- `RemotePanel*` を対象にしたスクリーンショットフィルタ（152 行目相当 `'src/components/panels/RemotePanel*'`）を除去。これにより対象が空になるフィルタ/ジョブが壊れないこと（フィルタ一覧から該当エントリのみ削除）。
- remote 削除後も CI がクリーンに通る構成にする（`pnpm build` が remote を呼ばない状態に整合）。

### バックエンド削除（`src-tauri/src/`）

#### command 表層（`adaptor/controller/command/`）

`tauri::generate_handler!` 相当の登録（`command/mod.rs`）と各 command 定義から、remote フロント専用で削除後に呼び出し元ゼロになるものを除去する。下層 usecase/gateway/domain は**保持**する（削除は controller 表層に限定）。

- `ws_server::commands`（`command/mod.rs` 135–139 行目相当の登録）:
  - `start_server` / `stop_server`（Tauri command ラッパー。`start_server_core`/`stop_server_core` 本体は残置）。
  - `get_server_status`（既に呼び出し元ゼロ）/ `get_server_info`。
  - `update_terminal_startup_command`。
- `remote_access::commands`（`command/remote_access/commands.rs`、`mod.rs` 218 行目相当 `remote_access::register`）:
  - `get_network_info` / `get_connection_qr`。`remote_access::register` 自体が空になる場合はモジュール登録も整理。
- `app_config::commands`（`command/app_config/commands.rs`）:
  - `get_server_config` / `update_server_port` / `regenerate_token` / `get_remote_config` / `update_remote_config`（remote 起動 UI・SettingsModal Remote タブ専用）。
  - 同モジュール内のデスクトップ用 command（remote 無関係なもの）は保持。

> **決定 B（スコープ波及・確定）**: requirements が command として明示列挙したのは `start_server`/`stop_server` のみだが、`src/remote/`・RemotePanel 経路・SettingsModal Remote タブの削除により上記 command 群はすべて**呼び出し元ゼロ**になる。ユーザー合意「使わないものは全て削除・再利用するものは残す」に従い、これらの **command 表層（controller 層のラッパー）を全て削除**し、A1 で再利用しうる **usecase/gateway/domain は温存**する（ユーザー確認済み：2026-06-20）。`start_server_core`/`stop_server_core` と shell（案X）は明示的に温存対象。

#### tray（`src-tauri/src/tray.rs`）

- メニュー ID `START_SERVER`/`STOP_SERVER`（12–17 行目相当）。
- メニュー項目 `Start Server`/`Stop Server`（28–31 行目相当）。
- ハンドラ `handle_start_server`（107–138 行目相当）/`handle_stop_server`（140–146 行目相当）と `handle_menu_event` のルーティング分岐（77–82 行目相当）。
- `update_tray_menu(server_running)`（148–153 行目相当）と、`server-status-changed` を購読してメニュー状態を更新する `listen_server_status`（155–166 行目相当）。
- Quit ハンドラ内でサーバ停止を呼ぶ箇所（99 行目相当）。tray からの `start_server_core`/`stop_server_core` 呼び出しが全て消えること。

#### 自動起動配線（`src-tauri/src/lib.rs`）

- `config.remote.auto_start` を見て `start_server_core` を spawn する setup 内ブロック（375–392 行目相当）を削除。

#### config（`src-tauri/src/adaptor/gateway/app_config/config_models.rs` ＋ domain）

- `RemoteSection { auto_start, auto_start_on_lan }`（70–75 行目相当）を削除。`AppConfig` の `remote` フィールドと、それを参照する変換・テスト（`config_models.rs` の domain⇔model 変換、`domain/app_config/value_objects` の `RemoteConfig`）を併せて削除。
- `AppSection.last_bind_ip`（90 行目相当）を削除。これに伴い `start_server_core` 末尾の `config.app.last_bind_ip = ...; save(...)` ブロック（`commands.rs` 130–134 行目相当）を削除する（**仮定 C**: これは「温存に必要な最小限の調整」の範囲内とみなす。core 本体ロジックの作り変えではなく、削除フィールドへの代入除去）。
- `ServerSection { bind, port, tls }`（152 行目以降相当）は**残置**。残置 shell（`http.rs` の bind/port 使用）と `start_server_core`（`cfg.server.bind`/`cfg.server.tls`）が使用するため（**仮定 D**: requirements の「サーバーポート等の remote 起動専用設定」は、UI から port を変更する command `update_server_port` の削除を指し、`ServerSection` 自体は残置 core/shell の動作に必要なため保持する）。

#### ws req/resp 表層（`src-tauri/src/ws_server/` ＋ `protocol/` または `adaptor/protocol/`）

- `protocol/mod.rs`（`WsMessage` 定義）から削除対象 variant とその request/response 構造体定義を除去（詳細は「データモデル」節）。
- `ws_server/routing.rs`: req/resp と inbound PTY 制御の match 分岐（18–100 行目相当）を削除。未知メッセージ既定の `_ => Error(INVALID_MESSAGE)`（101–105 行目相当）は**残置**し、縮退後 shell の inbound 受理はこれに集約。関連テスト（削除 variant のルーティングテスト）を除去、`test_route_unknown_message_returns_error` は残置。
- `ws_server/handlers.rs`: req/resp ハンドラ実装（`handle_backend_list_request`、`handle_agent_*_request` 群、`handle_review_*_request` 群、`pty_handler` の spawn/kill/output/input/resize ハンドラ）を削除。これらが呼んでいた usecase/gateway/infrastructure（`RepositoryUsecase`・`SessionStore`・`AgentProcessMap`・`ReviewCommentStore`・`AgentBackendRegistry` 等）は**保持**。
- `ws_server/validation.rs`: req/resp 専用バリデーションがあれば削除。パス検証（`normalize_path`/`validate_relative_path`）が push/認証/A1 で使われるなら残置、req/resp 専用で他参照が無ければ削除（実装時に参照を確認して判定）。
- `ws_server/commands.rs`: `start_server_core`/`stop_server_core` は残置（`#[allow(dead_code)]` 付与）。`update_terminal_startup_command` command と、それが呼ぶ `WsServerState::set_terminal_startup_command` は、参照が消えるなら併せて整理（残置 shell の最小性を壊さない範囲で）。

#### push/認証 shell（残置・案X）

- `ws_server/http.rs`・`auth.rs`・`rate_limit.rs`・`session.rs`（認証フェーズ・broadcaster setup・forward task）。
- `ws_bridge.rs` の `WsBroadcaster` 全 API。
- push variant（`*Sync`/`PtyOutput`/`PtyExit`/`PtyReady`）と認証 variant（`AuthChallenge`/`AuthResponse`/`AuthResult`）と `Error`。

## アーキテクチャと責務分割

```
[削除] src/remote/ (RemoteApp)  ──ws──╳  [縮退] ws_server (認証+push shell)
[削除] RemotePanel/useRemote* ─invoke─╳  [削除] command 表層(remote専用)
[削除] SettingsModal "Remote"タブ ─────╳  [削除] tray Start/Stop, lib.rs auto-start, config remote
                                          [残置] start_server_core/stop_server_core (#[allow(dead_code)])
                                          [残置] http/auth/rate_limit/WsBroadcaster (案X)
[保持] デスクトップ全機能 ─invoke/emit/listen→ [保持] usecase/gateway/domain/infrastructure
```

- 削除は「クライアント表層（フロント）」と「ws req/resp 表層 + remote 専用 command 表層 + 起動トリガー」に限定する。
- クリーンアーキテクチャの依存方向（`infrastructure → gateway → domain ← usecase ← controller`）に沿い、削除は controller/protocol 表層側から行い、usecase 以下は保持する（behavior Rule 4 / 下層ロジック保持）。
- 残置 core/shell は呼び出し元ゼロになるため `#[allow(dead_code)]` で温存する（**仮定 E**: 温存手段は `cfg` ではなく `#[allow(dead_code)]` を採用。理由：A1 で `cfg` フラグ無しに素直に再配線でき、ビルド構成を分岐させない。`clippy -D warnings` を通す）。

## データモデル / 型

### 削除する `WsMessage` variant（req/resp + inbound PTY 制御）

req/resp 対（削除）:

- PTY: `PtySpawnRequest`/`PtySpawnResponse`、`PtyKillRequest`/`PtyKillResponse`、`PtyOutputRequest`。
- inbound 制御（送信元消滅により削除）: `PtyInput`、`PtyResize`。
- Branch: `BranchInfoRequest`/`BranchInfoResponse`。
- Worktree: `WorktreeListRequest`/`WorktreeListResponse`、`WorktreeSelectRequest`/`WorktreeSelectResponse`。
- Backend: `BackendListRequest`/`BackendListResponse`。
- Agent（req/resp 全対）: `AgentSessionStart*`、`AgentSessions*`、`AgentSessionGet*`、`AgentMessage*`、`AgentInterrupt*`、`AgentQueueCancel*`、`AgentSlashCommands*`、`AgentMentionFiles*`、`AgentImagePrepare*`、`AgentPermissionResponse*`、`AgentModelSet*`、`AgentPermissionModeSet*`。
- Review: `ReviewListRequest`/`ReviewListResponse`、`ReviewGetRequest`/`ReviewThreadResponse`、`ReviewCreateRequest`、`ReviewAppendCommentRequest`、`ReviewResolveRequest`、`ReviewHistoryRequest`/`ReviewHistoryResponse`。

各 variant に紐づく request/response 構造体定義・`serde` 派生・関連テストも除去する。

> **仮定 F（`PtyReady`/`PtyKillResponse` の扱い）**: 調査では `PtyReady`/`PtyKillResponse` を push 系として残置候補に挙げているが、`PtyKillResponse` は `PtyKillRequest` の応答（req/resp 対）であり送信元 command 削除で不要化する可能性が高い。`PtyReady` は spawn 完了 push として A1 で再利用しうる。実装時に「残置 push（broadcast 単独で送られるか）」「req/resp 応答（削除）」を `WsBroadcaster` 送信経路の有無で判定し確定する。push 単独送信があるもののみ残す。

### 残置する `WsMessage` variant

- push: `BranchListSync`、`WorktreePrStatusSync`、`AgentStateSync`、`WorkflowStateSync`、`AgentStreamSync`、`PtyOutput`、`PtyExit`、（`PtyReady` は仮定 F で確定）。
- 認証: `AuthChallenge`、`AuthResponse`、`AuthResult`。
- エラー: `Error`。

### config 型

- 削除: `RemoteSection`、`AppConfig.remote`、`AppSection.last_bind_ip`、domain `RemoteConfig`。
- 残置: `ServerSection`（bind/port/tls）。
- 後方互換: app config は `deny_unknown_fields` を使用していない（確認済み）ため、既存 TOML に残る `[remote]` セクション・`last_bind_ip` キーは deserialize 時に**無視**される。マイグレーション処理は不要（**仮定 G**）。

## 処理フロー（縮退後の ws inbound）

1. クライアント接続 → 認証ハンドシェイク（`AuthChallenge` → `AuthResponse` → `AuthResult`）。`auth.rs`/`rate_limit.rs` は従来どおり。
2. 認証成功後、サーバ→クライアントは push（`*Sync`/`PtyOutput`/`PtyExit`/`PtyReady`）を broadcast。`session.rs` の forward task と `WsBroadcaster` を使用。
3. クライアント→サーバの inbound メッセージ（削除済み req/resp 相当・未知）は `routing.rs` の `_ => Error(INVALID_MESSAGE)` で応答し、接続は維持（behavior Rule 3・現行踏襲）。

## エラー処理

- 縮退後 shell の inbound 受理は、認証ハンドシェイク以外すべて `Error(code: "INVALID_MESSAGE")` 応答 + 接続維持（切断・無言破棄しない）。既存テスト `test_route_unknown_message_returns_error` と整合。
- config 読み込み：`[remote]` 等の旧キーは serde で無視。読み込み失敗時の既定動作（`Default`）は変更しない。
- 残置 core/shell のエラー処理（TLS/bind 失敗、認証失敗の rate limit）は変更しない（案X）。

## テスト方針

- **削除に伴うテスト除去**: `src/remote/` 配下テスト、`RemotePanel.test.tsx`、`useRemoteServer.test.ts`、`useRemoteAutoStart.test.ts`、`useRemoteConfig` 関連テスト、`SettingsModal.test.tsx` の Remote タブ assertion、ws req/resp ハンドラ/ルーティングの該当テスト、削除 command のテスト。
- **残置の回帰テスト（既存緑維持）**:
  - ws: `test_route_unknown_message_returns_error`（縮退後も Error 応答）。認証・push 系の既存テスト。
  - config: `RemoteSection`/`last_bind_ip` 削除後も既存 config テスト（`ServerSection` 等）が緑。旧キーを含む TOML が無視される後方互換を最小テストで確認（**仮定 H**: 後方互換テストを 1 ケース追加するのは「削除に伴う回帰防止」の範囲内とし、スコープ拡大に当たらない）。
  - デスクトップ機能：invoke 経由の既存テストが緑（remote 削除の影響を受けないことの回帰確認）。
- **クリーンビルド/lint/CI**（behavior Rule 5）:
  - `pnpm build` / `pnpm lint` / `pnpm test`。
  - `cargo fmt --check` / `cargo clippy -- -D warnings`（温存 core/shell が `#[allow(dead_code)]` で警告を出さないこと）/ `cargo test`。
  - CI が remote 削除後にクリーンに通る。
- **参照切れ検査**: 削除対象シンボルへの参照切れ・未使用 import が残らないこと（behavior Rule 5 後半）。

## リスクと代替案

- **リスク 1: 削除波及の過不足**。remote 専用と判断した command/SettingsModal タブが、実はデスクトップから別経路で使われている可能性。→ 緩和：grep で呼び出し元ゼロを確認済み（`get_remote_config` は SettingsModal/useRemoteAutoStart/useRemoteConfig のみ、他は useRemoteServer/useRemoteAutoStart のみ）。実装前に再 grep で最終確認。
- **リスク 2: 残置 core の `#[allow(dead_code)]` 漏れ**で clippy 失敗。→ 緩和：`start_server_core`/`stop_server_core`/`WsServerState` 等、呼び出し元ゼロになる範囲を洗い出し付与。
- **リスク 3: `PtyReady`/`PtyKillResponse` の残置/削除誤判定**（仮定 F）。→ 緩和：`WsBroadcaster` 経由の push 送信有無で機械的に判定。
- **リスク 4: `last_bind_ip` 削除による core 改変**が「core を改変しない」制約と緊張（仮定 C）。→ 代替案：`last_bind_ip` を残置する案もあるが、requirements が明示的に削除を要求しているため削除を採用し、core 側は代入除去の最小調整に留める。
- **代替案（温存手段）**: `#[allow(dead_code)]` ではなく `#[cfg(feature = "...")]` でガードする案。→ 不採用：ビルド構成が分岐し A1 再配線が複雑化するため（仮定 E）。

## 仮定（本文中で明示したものの一覧）

- **A**: remote 専用依存は `html5-qrcode`・`@noble/hashes` のみ（`src/remote/` 外参照なしを grep 確認）。`@xterm/*` はデスクトップでも使用のため残置。
- **B**（確定）: remote フロント削除で呼び出し元ゼロになる command 表層（`get_network_info`/`get_connection_qr`/`update_server_port`/`regenerate_token`/`get_server_config`/`get_remote_config`/`update_remote_config`/`get_server_info`/`get_server_status`/`update_terminal_startup_command`）と SettingsModal "Remote" タブ・`useRemoteConfig` も「使わないものは全て削除」に従い削除。usecase/gateway/domain・残置 core/shell・`ServerSection` は温存（ユーザー確認済み：2026-06-20）。
- **C**: `last_bind_ip` 削除に伴う `start_server_core` の代入除去は「温存に必要な最小限の調整」に含む。
- **D**: `ServerSection`(bind/port/tls) は残置 core/shell が使用するため保持。requirements の「サーバーポート等」は `update_server_port` command を指すと解釈。
- **E**: 温存手段は `#[allow(dead_code)]` を採用（`cfg` 不採用）。
- **F**: `PtyReady`/`PtyKillResponse` の残置/削除は `WsBroadcaster` 経由 push 送信の有無で実装時に確定。
- **G**: app config は `deny_unknown_fields` 非使用のため旧 `[remote]`/`last_bind_ip` は無視され、config マイグレーション不要。
- **H**: 後方互換の最小テスト 1 ケース追加は回帰防止の範囲とする。

## Open Questions

なし。

Q1（起動トリガー周辺 command 表層 約10個 ＋ SettingsModal "Remote" タブ ＋ `useRemoteConfig` の削除範囲）は **「全て削除」で確定**（ユーザー確認済み：2026-06-20）。呼び出し元ゼロになる command 表層・SettingsModal "Remote" タブ・`useRemoteConfig` を削除し、A1 再利用分（usecase/gateway/domain・`start_server_core`/`stop_server_core`・shell（案X）・`ServerSection`）は温存する。決定 B 参照。
