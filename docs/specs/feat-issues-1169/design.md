# Design

## Source
- requirements.md
- behavior.md

## 概要

`app_config`（`src-tauri/src/config.rs`）と `mcp`（`src-tauri/src/mcp/`）の 2 ドメインを、`docs/architecture/` のクリーンアーキテクチャ規約（`infrastructure → adaptor → usecase → domain`、依存は内向きのみ）に従ったレイヤー構成へ一括移行する。先行事例 `notification`（自由関数 usecase）・`pty_session`（状態保持 gateway + generic trait）・`repository`/`code`/`workflow`（struct usecase + `wiring.rs` 合成）のパターンを踏襲する。

移行後は旧モジュール（`config.rs`、`mcp/`）を除去し（no-shim）、ドメインロジックがファイルシステム・rmcp/axum・Tauri へ直接依存しない状態にする。behavior.md で定義した観測可能な振る舞い（設定の参照／更新／永続化、トークン再生成、MCP サーバの起動／停止／状態取得、認証付きツール提供、外部エージェント向け MCP 設定の生成・管理）は維持する。

本移行の中核論点は次の 2 つであり、本書はこれらの解消方針を含む。

1. **config ↔ mcp の循環依存**: 現状 `config.rs` の `update_mcp_config` / `regenerate_mcp_token` が `crate::mcp::restart_mcp_server_if_running` を呼び、逆に `mcp` は `AppConfig` を参照している。
2. **`AppConfig` の広域参照**: `AppConfig`（`Mutex<ReleashConfig> + PathBuf`）は 38 ファイル・他ドメイン（notification / external_editor / workflow / repository 等）の gateway から `Arc<AppConfig>` で参照されている。これらは non-goal だが、型の再配置により追従が必要になる。

## 変更対象

### 除去するモジュール（no-shim）
- `src-tauri/src/config.rs`（約 1,391 行）
- `src-tauri/src/mcp/{mod.rs, server.rs, mcp_json.rs, state.rs, auth.rs}`（約 1,846 行）

### 新規追加（app_config）
- `src-tauri/src/domain/app_config/` — value_objects（各設定セクションの domain 型）、`repository.rs`（永続化 trait）、`services.rs`（トークン生成等の純粋ロジック）、`error.rs`、`mod.rs`
- `src-tauri/src/usecase/app_config/` — `usecase.rs`（Command）、`query_service.rs`（Query）、`dto.rs`、`error.rs`、`mod.rs`
- `src-tauri/src/adaptor/gateway/app_config/` — `repository_impl.rs`（TOML I/O + 状態保持）、`config_models.rs`（domain ↔ TOML 変換）、`mod.rs`
- `src-tauri/src/adaptor/controller/command/app_config/` — `commands.rs`（`#[tauri::command]` 群）、`mod.rs`（`register`）
- 必要に応じ `src-tauri/src/infrastructure/persistence/`（アトミック書込ヘルパ）

### 新規追加（mcp）
- `src-tauri/src/domain/mcp/` — entities（`McpServer` 等の状態）、value_objects（`AgentKind`、`McpConnectionInfo`、`McpServerStatus`、トークン検証規則）、`gateway.rs`（サーバ起動／停止・エージェント設定 I/O の trait）、`services.rs`（認証判定・設定内容生成の純粋ロジック）、`error.rs`、`mod.rs`
- `src-tauri/src/usecase/mcp/` — `lifecycle_usecase.rs`（start/stop/status/restart）、`agent_config_usecase.rs`（生成・削除・保存）、`query_service.rs`（プレビュー・一覧）、`dto.rs`、`error.rs`、`mod.rs`
- `src-tauri/src/adaptor/gateway/mcp/` — `server_impl.rs`（rmcp/axum ライフサイクル）、`agent_config_impl.rs`（エージェント設定ファイル I/O）、`mod.rs`
- `src-tauri/src/adaptor/controller/command/mcp/` — `commands.rs`、`mod.rs`（`register`）
- `src-tauri/src/infrastructure/external/mcp/` — rmcp `ServerHandler`（`ReleashMcpServer`）とツール定義、axum ルータ・認証ミドルウェアの薄いラッパー

### 変更（追従・配線）
- `src-tauri/src/lib.rs` — `config`/`mcp` の `mod` 宣言除去、AppConfig 相当状態の構築・managed state 登録の差し替え、MCP 自動起動・DI 配線の更新
- `src-tauri/src/adaptor/controller/command/mod.rs` — `register_all` に `app_config::register` / `mcp::register` を追加
- `src-tauri/src/adaptor/controller/wiring.rs` — `crate::config::AppConfig` 参照を新配置へ差し替え
- `crate::config::AppConfig` / `ReleashConfig` の各種 Section（`NotifySection` 等）を参照している他ドメイン gateway・ws_server・sentry_integration 等のインポート追従

## アーキテクチャと責務分割

### app_config

| 層 | 責務 | 主な要素 |
|---|---|---|
| domain | 設定値の保持規則・トークン生成規則・永続化の抽象 | 各設定 Section の domain 型（serde を持たない）、`ConfigRepository` trait（`load` / `save`）、`services::generate_token`（48 文字英数字） |
| usecase | 設定の取得・更新・トークン再生成の業務手順 | `AppConfigUsecase`（Command: update_*, regenerate_token, regenerate_mcp_token）、`AppConfigQueryService`（Query: get_server / get_mcp / get_app / get_remote / get_workflow / get_telemetry / get_crash_reporting）、入出力 DTO |
| adaptor/gateway | TOML 永続化と in-memory キャッシュ | `ConfigRepositoryImpl`（`Mutex<ReleashConfigModel> + PathBuf` を保持、`toml` シリアライズ／デシリアライズ、アトミック書込、空トークンの初期生成）、`config_models`（domain ↔ TOML モデル変換） |
| adaptor/controller | Tauri コマンド入口 | `commands`（薄い委譲）、`register` |
| infrastructure | ファイル I/O プリミティブ | アトミック書込（tmp → rename、Unix で 0o600） |

- 現状の `AppConfig`（`Mutex<ReleashConfig> + PathBuf`）は「in-memory キャッシュ + 永続化」を担う gateway 的状態であり、`ConfigRepositoryImpl` として `adaptor/gateway/app_config/` へ再配置する（pty_session の `PtySessionRuntimeGateway` が状態を持つ前例に倣う）。
- `generate_token`（長さ・文字種というビジネスルール）は domain service に置く（**仮定 1** 参照）。
- TOML 構造体（`ReleashConfig` とその Section 群、serde derive 付き）は gateway 層の永続化モデル（`config_models`）とし、domain 型はこれと分離した serde なしの型にする。1:1 の写像でも、フロント／TOML の都合を domain に漏らさない規約（DOMAIN.md・GATEWAY.md）に従う。

### mcp

| 層 | 責務 | 主な要素 |
|---|---|---|
| domain | MCP サーバの状態・認証規則・エージェント設定の生成規則 | `McpServer` エンティティ（running/port/token）、`AgentKind`（Claude/Codex/Gemini/Cursor）、`McpConnectionInfo` / `McpServerStatus`、トークン一致判定、`gateway.rs`（`McpServerGateway` start/stop/status、`AgentConfigGateway` 生成/削除/検出）、設定内容生成の純粋ロジック |
| usecase | 起動／停止／状態取得・エージェント設定の生成・管理 | `McpLifecycleUsecase`、`McpAgentConfigUsecase`、`McpQueryService`（プレビュー・設定済み一覧）、DTO |
| adaptor/gateway | rmcp/axum ライフサイクル・設定ファイル I/O | `McpServerGatewayImpl`（rmcp `StreamableHttpService` + axum、`CancellationToken` で停止）、`AgentConfigGatewayImpl`（`.claude.json` / `.codex/config.toml` / `.gemini/settings.json` / `.cursor/mcp.json` への merge 書込・削除・検出） |
| adaptor/controller | Tauri コマンド入口 | `commands`、`register` |
| infrastructure | rmcp サーバ実体・認証ミドルウェア | `ReleashMcpServer`（`ServerHandler` 実装、ツール `worktrees_list` / `create_workspace` / `read_file`）、axum 認証ミドルウェア（Bearer 検証） |

- MCP ツール（`worktrees_list` 等）は `repository_usecase` / `code_usecase` を呼ぶ既存設計を維持する。rmcp サーバ実装は README が `infrastructure/external/ # ... MCP` を明記しているため `infrastructure/external/mcp/` に置く（infrastructure → usecase は内向き依存で規約適合）。
- MCP サーバの「起動状態」を保持する `McpServerHandle`（Tauri managed state、`Arc<Mutex<...>>` 群）は、`McpServerGatewayImpl` が内部に保持する状態として再配置する。

### config ↔ mcp 循環依存の解消

usecase 同士を相互依存させず、**controller 層でオーケストレーション**する。

- `update_mcp_config` / `regenerate_mcp_token` コマンドは、`AppConfigUsecase` で設定を保存した後、続けて `McpLifecycleUsecase::restart_if_running` を呼ぶ。設定保存と MCP 再起動の順序制御を controller（composition root に近い入口）に置くことで、`app_config` usecase が `mcp` を知らず、`mcp` usecase が `app_config` を一方向に参照する（mcp → app_config）構成にする。
- `mcp` は起動時に mcp_port / mcp_token を必要とするため、`app_config` の Query（または ConfigRepository）に依存してよい（mcp → app_config の一方向）。
- `mcp_json.rs` の `save_and_generate_mcp_configs`（設定保存 → MCP 再起動 → 各エージェント設定生成）も同様に controller でオーケストレーションする。

### AppConfig 広域参照の扱い（決定: 全面追従）

`AppConfig`（`Arc<AppConfig>`）は他ドメイン（non-goal: notification / external_editor / workflow / repository / ws_server / sentry_integration 等、計 38 ファイル）の gateway から広く参照されている。**本移行ではこれらを新しい `app_config` の抽象（`ConfigRepository` trait と `ConfigRepositoryImpl`）へ全面的に追従させる**（ユーザー決定）。

- 他ドメイン gateway は `Arc<AppConfig>`（具体型）への直接依存をやめ、`app_config` ドメインが定義する抽象（`Arc<dyn ConfigRepository>`、または読み取り用の Query ポート）を DI で受け取る形へ移行する。gateway → domain（app_config の trait）は内向き依存で規約適合。
- 旧 `ReleashConfig` の各 Section を直接参照していたコードは、`app_config` の domain 型／DTO 経由のアクセスへ置き換える。
- この追従は non-goal ドメインのコードへ波及するが、振る舞いは変えない範囲に限定する（型・依存の付け替えのみ）。波及範囲・作業量・リスクの増大を許容したうえで、依存方向を完全に内向きへ揃えることを優先する。

## データモデルまたは型

### app_config（domain 値オブジェクト、serde なし）
- `ServerConfig`（bind, port, hook_port, token, mcp_port, mcp_token, tls, notify）
- `TlsConfig`、`NotifyConfig`（既存 `domain/notification` の `NotifyConfig` と重複しないよう所有を整理）、`DesktopNotifyMode`
- `TelemetryConfig`（crash_reporting）、`AppSettings`、`AgentShortcutConfig`、`AgentsConfig`、`RemoteConfig`、`WorkflowConfig`
- domain サービス: `generate_token() -> String`

### app_config（gateway 永続化モデル、serde あり）
- `ReleashConfigModel` とその Section 群（現 `ReleashConfig` 等を移設、`#[serde(default)]` による後方互換を維持）
- domain ↔ model 変換関数（`*_to_domain` / `*_to_model`）

### app_config（usecase DTO、serde camelCase）
- 各 Query の Response（現 `get_server_config` 等の戻り値に対応）。`McpConfig{port, token}` 等。

### mcp
- domain: `McpServer`（`running: bool`, `port: Option<u16>`, `token`）、`AgentKind`、`McpConnectionInfo{url, token}`、`McpServerStatus{running, port}`
- usecase DTO: 各コマンド戻り値（接続情報・状態・設定済みエージェント一覧・プレビュー内容）
- gateway: rmcp ツールパラメータ型は `infrastructure/external/mcp/` に閉じる

### エラー型
- `domain/app_config/error.rs`・`domain/mcp/error.rs`（thiserror）
- `usecase/*/error.rs`（`UsecaseError`、domain エラーから `#[from]`）
- controller 戻り値は `AppError`（`other/error.rs`、現状の文字列等価表現を維持）に集約

## 処理フロー

### 設定の参照（例: get_server_config）
controller `get_server_config` → `AppConfigUsecase`（内部 `AppConfigQueryService`）→ `ConfigRepositoryImpl::load`（in-memory キャッシュ）→ domain → DTO → AppError 変換して返却。

### 設定の更新と永続化（例: update_server_port）
controller `update_server_port` → `AppConfigUsecase::update_server_port` → `ConfigRepositoryImpl::save`（キャッシュ更新 + TOML アトミック書込）。再起動後も `load` で読み戻せる（behavior「設定が永続化される」）。

### トークン再生成（例: regenerate_mcp_token）
controller → `AppConfigUsecase::regenerate_mcp_token`（`services::generate_token` で新トークン生成 → 保存）→ controller が続けて `McpLifecycleUsecase::restart_if_running` を呼ぶ → 新トークン返却。

### 設定ファイル不在時
`ConfigRepositoryImpl` 初期化時、ファイルが存在しなければ既定値を採用し、空トークンを生成して書き戻す（behavior「設定ファイルが存在しない場合でも設定を利用できる」）。読み取り専用ローダ（現 `read_config_if_exists`、副作用なし）の用途が残る場合は Query 経路として保持する。

### MCP サーバ起動／停止／状態
controller `start_mcp_server` → `McpLifecycleUsecase::start`（`app_config` から mcp_port/token 取得 → `McpServerGatewayImpl::start` が rmcp/axum を bind、認証ミドルウェア付与）。`stop` は `CancellationToken` で graceful shutdown。`get_mcp_server_status` / `get_mcp_connection_info` は Query。

### 認証付きツール提供
外部 MCP クライアント接続 → axum 認証ミドルウェアが Bearer トークンを `app_config` の mcp_token と照合 → 一致で通過し rmcp `ServerHandler` がツール一覧／実行を提供、不一致で 401（behavior「認証付きでツールを提供する」）。

### 外部エージェント MCP 設定の生成・管理
controller `save_and_generate_mcp_configs` → `AppConfigUsecase` で mcp_port/token 保存 → `McpLifecycleUsecase::restart_if_running` → `McpAgentConfigUsecase::generate`（`AgentConfigGatewayImpl` が各エージェント設定ファイルへ merge 書込）。プレビュー・設定済み一覧は `McpQueryService`、削除は `McpAgentConfigUsecase::remove`。

## エラー処理

- domain / usecase は固有エラー型を返し、`#[from]` で連鎖。controller で `AppError`（文字列等価表現）へ集約しフロント／リモートへ返却する。`other/error.rs` の既存契約（オブジェクトに包まずメッセージ文字列を返す）を維持する。
- TOML パース失敗は `Err`、ファイル不在は既定値（不在を I/O エラーと取り違えないため `try_exists` を使う現状実装を踏襲）。
- MCP 起動失敗（bind 失敗等）はエラーを返し、サーバ状態を矛盾なくクリアする。認証失敗は 401。
- アトミック書込失敗時は一時ファイルを掃除する現状動作を維持する。

## テスト方針

TEST.md に従い、命名は `test_{業務機能}_{条件と期待結果}`（業務機能は日本語）、Given-When-Then 構造。

- **domain（必須）**: `generate_token`（長さ・文字種・異なる値の生成）、トークン一致判定、`AgentKind` の設定パス・設定内容生成、`McpServer` 状態遷移、domain ↔ model 変換の同値性。
- **usecase（必須）**: モック ConfigRepository / MckServerGateway / AgentConfigGateway を用い、update→get で新値が得られる、regenerate で旧値と異なる、不在時に既定値、start/stop/status の状態遷移、restart_if_running の条件分岐。
- **adaptor/gateway（必須）**: `ConfigRepositoryImpl` の TOML ラウンドトリップ（`tempdir` 実ファイル）、アトミック書込、空トークン初期生成、`AgentConfigGatewayImpl` の merge/削除/検出（既存他サーバエントリの保持）。`McpServerGatewayImpl` は bind/stop をロジック範囲で。
- **controller / infrastructure（柔軟）**: Tauri / rmcp 依存は書ける範囲で。循環依存解消のオーケストレーション（保存→再起動の順序）は usecase 分割により検証可能な形にする。
- 既存 CI（フロント lint/test/build、Rust fmt/clippy/test）を通過させる。

## リスクと代替案

- **広域参照の追従コスト（全面追従を採用）**: `AppConfig` 参照 38 ファイルを新抽象（`ConfigRepository`）へ全面追従させるため、non-goal ドメインの gateway にも変更が波及しビルド全体に影響する。リスク低減のため、(1) `app_config` のドメイン・抽象・gateway を先に確立し、(2) 他ドメインを 1 ドメインずつ新抽象へ付け替え、各段でビルド・テストを通す、という段階移行を行う。各ステップは振る舞いを変えない型・依存の付け替えに限定する。
- **循環依存解消の代替案**: (A) controller オーケストレーション（採用）/ (B) `app_config` が再起動コールバックを保持し DI で注入 / (C) イベントバス経由。(A) は依存方向が最も明快で先行事例（pty_session の usecase 共有）とも整合するため採用。
- **rmcp サーバの配置代替案**: (A) `infrastructure/external/mcp`（採用、README 準拠）/ (B) `adaptor/controller/handler/mcp`（WebSocket handler との一貫性）。外部入口という性質は (B) 寄りだが、rmcp/axum トランスポートは外部ライブラリ実体であり README の明示配置に従い (A) を採用。Open Question 2 で確認余地。
- **domain への rand 依存**: `generate_token` を domain service に置くと rand が domain に入る。DOMAIN.md の禁止列挙（tauri/git2/tokio/reqwest/sqlx）に rand は含まれないが、純粋性の観点で議論余地（仮定 1 / Open Question 2）。
- **設定セクションの所有重複**: `NotifyConfig` / `DesktopNotifyMode` は既存 `domain/notification` にも存在する。所有ドメインを一本化し重複定義を避ける（横断原則「同一操作の実装は 1 つ」）。

## 仮定

1. **トークン生成の配置**: `generate_token`（48 文字英数字というビジネスルール）は `domain/app_config/services.rs` に置く。乱数生成（rand）は外部リソースではなく純粋計算ライブラリとみなし domain で使用する。Open Question 1 の回答次第で gateway 側へ移す。
2. **永続化モデルと domain 型の分離**: 現 `ReleashConfig`（serde 付き TOML 構造体）は gateway の永続化モデルとし、domain には serde を持たない別型を置く。1:1 写像でも DOMAIN.md / GATEWAY.md の「フロント・永続化都合を domain に漏らさない」規約に従う。
3. **rmcp サーバ配置**: `ReleashMcpServer`（ServerHandler とツール群）・認証ミドルウェアは `infrastructure/external/mcp/` に置く（README の `infrastructure/external/ ... MCP` に準拠）。
4. **MCP ツールの usecase 委譲は現状維持**: ツールは `repository_usecase` / `code_usecase` を呼ぶ。本移行で MCP プロトコル仕様・ツール定義は変更しない（Non-goals）。
5. **循環依存解消は controller オーケストレーション**: 設定保存 → MCP 再起動の順序制御を controller 入口に置き、usecase の相互依存を作らない（mcp → app_config の一方向のみ）。
6. **DI 配線スタイル**: `app_config` は共有状態を持つため `ConfigRepositoryImpl` を managed state（または AppState）として配線する。`mcp` lifecycle は struct usecase + `wiring.rs` 合成に寄せる。具体的な AppState 拡張範囲は先行事例（repository/code/workflow）に合わせる。
7. **読み取り専用ローダの保持**: 副作用のない設定読み取り（現 `read_config_if_exists`、CLI 等の観測用途）は Query 経路として維持する。

## Open Questions

なし（すべて解消済み）。

解消した論点と決定:

1. **`AppConfig` 広域参照の追従範囲** → **全面追従**。他ドメイン（non-goal）の gateway を新しい `app_config` の抽象（`ConfigRepository`）へ移行し、`Arc<AppConfig>` 具体型への直接依存を解消する。詳細は「AppConfig 広域参照の扱い（決定: 全面追従）」を参照。
2. **トークン生成の配置・rmcp の配置** → **仮定どおり**。`generate_token` は `domain/app_config/services.rs`（rand を domain で使用）、rmcp サーバ実装は `infrastructure/external/mcp/`（README 準拠）に置く。
