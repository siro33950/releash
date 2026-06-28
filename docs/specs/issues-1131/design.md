# Design

## 概要

WebSocket server 本体（`ws_server/`）、broadcast bridge（`ws_bridge.rs`）、agent status event 変換
（`agent_status_events.rs`）を、clean architecture の層へ移設する構造リファクタリングである。
対外契約（message 名・payload JSON shape）と transport 振る舞い（auth / reconnect / resync /
PTY replay / stream buffering / push notification）は完全に維持し、コードの物理配置と import 経路のみを変更する。

本 design は requirements.md / behavior.md が design.md に委ねた以下の具体を確定する。

- `ws_server/` 配下各ファイルの分割粒度と移設先。
- `WsServerState` / `WsServerHandle` / `StartServerResult` / server 起動・停止 plumbing の所属。
- `route_message`（handler）と `WsServerState`（middleware）の結合をどう解消するか。
- `agent_status_events.rs` の最終配置。
- `WsBroadcaster` の配置。

確定方針（要約）:

- transport 機構（TCP accept / TLS / HTTP upgrade / auth handshake framing / forward task / rate limit /
  server lifecycle state）は `infrastructure/middleware/` へ移す。
- protocol message → usecase の request dispatch（`route_message`）は `adaptor/controller/handler/` へ移す。
- outbound broadcaster（`WsBroadcaster`）は `adaptor/gateway/shared/` へ移す。
- usecase/domain state → push payload 変換（`emit_agent_status_changes`）は `adaptor/presenter/` へ移す。

## 変更対象

### 削除されるルート直下 module

- `src-tauri/src/ws_server/`（`mod.rs` / `auth.rs` / `commands.rs` / `http.rs` / `rate_limit.rs` / `routing.rs` / `session.rs`）
- `src-tauri/src/ws_bridge.rs`
- `src-tauri/src/agent_status_events.rs`

### 新規作成する module

- `src-tauri/src/infrastructure/middleware/`（`mod.rs` / `auth.rs` / `rate_limit.rs` / `http_upgrade.rs` / `session.rs` / `server_control.rs`）
- `src-tauri/src/adaptor/controller/handler/`（`mod.rs`）
- `src-tauri/src/adaptor/gateway/shared/ws_broadcaster.rs`
- `src-tauri/src/adaptor/presenter/agent_status.rs`

### import 経路を更新する既存ファイル

- `src-tauri/src/lib.rs`（composition root）
- `src-tauri/src/infrastructure/mod.rs`（`pub(crate) mod middleware;` 追加）
- `src-tauri/src/adaptor/controller/mod.rs`（`pub(crate) mod handler;` 追加）
- `src-tauri/src/adaptor/gateway/shared/mod.rs`（`pub mod ws_broadcaster;` 追加）
- `src-tauri/src/adaptor/presenter/mod.rs`（`pub(crate) mod agent_status;` 追加）
- `src-tauri/src/adaptor/gateway/repository/state.rs`（`WsBroadcaster` import path）
- `src-tauri/src/adaptor/gateway/workflow/state_notification_gateway.rs`（`WsBroadcaster` + `emit_agent_status_changes`）
- `src-tauri/src/adaptor/gateway/pty_session/backend_impl.rs`（`WsBroadcaster`）
- `src-tauri/src/adaptor/controller/agent_status_wiring.rs`（`WsBroadcaster` + `emit_agent_status_changes`）
- `src-tauri/src/infrastructure/agent_session/runtime/bridge_common/stream_emit.rs`（`WsBroadcaster`）
- `src-tauri/src/infrastructure/agent_session/runtime/bridge_common/shared.rs`（`WsBroadcaster` + `emit_agent_status_changes`）
- `src-tauri/src/usecase/workflow/mod.rs`（依存ガードテストの forbidden-list を更新。後述）

## アーキテクチャと責務分割

### 配置先と現コードの対応

| 現在 | 移設先 | 責務 |
|---|---|---|
| `ws_server/routing.rs` の `route_message` | `adaptor/controller/handler/mod.rs` | protocol message → usecase request dispatch |
| `ws_server/auth.rs` | `infrastructure/middleware/auth.rs` | HMAC challenge 生成 / 検証（純粋 transport concern） |
| `ws_server/rate_limit.rs` | `infrastructure/middleware/rate_limit.rs` | 認証失敗カウント / IP block |
| `ws_server/http.rs` | `infrastructure/middleware/http_upgrade.rs` | TCP bind / TLS accept / HTTP upgrade / security header / accept loop |
| `ws_server/session.rs` | `infrastructure/middleware/session.rs` | WS session lifecycle（handshake framing / forward task / PTY replay） |
| `ws_server/commands.rs` の `start_server_core` / `stop_server_core` / `ServerStatusPayload` | `infrastructure/middleware/server_control.rs` | server 起動・停止 orchestration と lifecycle state mutation |
| `ws_server/mod.rs` の `WsServerState` / `WsServerHandle` / `StartServerResult` | `infrastructure/middleware/mod.rs` | server runtime DI context と lifecycle handle |
| `ws_bridge.rs` の `WsBroadcaster` 一式 | `adaptor/gateway/shared/ws_broadcaster.rs` | outbound broadcaster（queue / collapse / forward） |
| `agent_status_events.rs` の `emit_agent_status_changes` | `adaptor/presenter/agent_status.rs` | usecase status change → Tauri emit + WS push payload 整形・送出 |

### 層責務の確定理由

- **handler（`route_message`）**: ResyncStream を `usecase::agent_session::session::resync_streaming_message`
  へ変換するのは「protocol → usecase 変換」そのものであり、behavior.md の Scenario
  「WebSocket routing / request handler entry → adaptor/controller/handler/」に対応する。CONTROLLER.md の
  `handler/` 規約に従い、ルーティング dispatch を `handler/mod.rs` に置く。現状の対象 message は
  ResyncStream 1 種のみのため、まず `handler/mod.rs` に集約し、ドメイン別 submodule への分割は将来の追加時に行う。
- **middleware（transport 機構）**: auth handshake の framing、rate limit、HTTP upgrade、TLS、accept loop、
  forward task、PTY replay、server lifecycle state は「純粋 transport concern」であり、infrastructure に属する。
  requirements.md が新規作成を明記した `infrastructure/middleware/` に集約する。
- **gateway（`WsBroadcaster`）**: requirements.md / behavior.md が「outbound broadcaster → adaptor/gateway/」と
  明示する。多数の gateway（repository / workflow / pty）と infrastructure / controller が共有する outbound
  primitive のため、`adaptor/gateway/shared/` に置く。
- **presenter（`emit_agent_status_changes`）**: `AgentStatusChanges`（usecase 型）を `AgentStateSync`
  （protocol payload）等へ変換する責務であり、presenter（レスポンス整形）に対応する。

### `route_message` と `WsServerState` の結合解消（重要な設計判断）

現状 `route_message(msg: &WsMessage, state: &WsServerState)` は infrastructure 側の型 `WsServerState` に
依存している。`route_message` を `adaptor/controller/handler/` へ移すと、handler が infrastructure 型を import
することになり層方向が乱れる。これを避けるため、`route_message` のシグネチャを必要な協力者のみを受け取る形へ変更する。

```rust
// adaptor/controller/handler/mod.rs
pub(crate) async fn route_message(
    msg: &WsMessage,
    broadcaster: &WsBroadcaster,                     // adaptor/gateway/shared
    resync_read_model: &dyn AgentStreamResyncReadModel, // usecase
) -> Option<WsMessage>
```

- handler の依存は `adaptor/gateway`（broadcaster）と `usecase`（read model trait）のみとなり、
  infrastructure を参照しない。
- middleware の `session.rs` は `WsServerState` のフィールドを取り出して
  `route_message(&ws_msg, &state.broadcaster, state.stream_resync_read_model.as_ref())` と呼ぶ。
  これは「WS 受信フレーム（infrastructure）→ handler 呼び出し」という inbound 経路であり、
  framework → controller の正方向であるため許容される。
- この変更は serialize 結果・分岐・エラーコードを一切変えない。`route_message` の内部ロジックは現状のままで、
  `state.broadcaster` / `state.stream_resync_read_model` への参照を引数参照へ置換するだけである。

`WsServerState`（broadcaster / pty_replay_reader / app_config / stream_resync_read_model / rate_limits /
active_connection / tls_enabled）は middleware 内（`http_upgrade.rs` / `session.rs`）でのみ使用されるため、
`infrastructure/middleware/mod.rs` に閉じる。フィールドは現状の private を維持する（子 module から ancestor の
private field へアクセス可能なため accessor 追加は不要）。

### 依存方向

- domain / usecase は adaptor / infrastructure を import しない（要件の唯一の hard constraint を維持）。
- 移設後の参照方向:
  - `infrastructure/middleware` → `adaptor/controller/handler`（inbound dispatch）、`adaptor/gateway`（broadcaster）、`usecase`（read model / resync）。
  - `adaptor/controller/handler` → `adaptor/gateway`（broadcaster）、`usecase`。
  - `adaptor/presenter/agent_status` → `adaptor/protocol`、`usecase::agent_session::status`、`adaptor/gateway`（broadcaster）。
  - `adaptor/gateway/shared/ws_broadcaster` → `adaptor/protocol`、`other::telemetry`。

## データモデルまたは型

新規型は導入しない。既存型を別 module へ移すのみで、定義・フィールド・derive・serde 属性は不変。

- `WsServerHandle` / `WsServerState` / `StartServerResult` / `ServerStatusPayload` / `RateLimitEntry`：
  定義そのまま、可視性は移設先 module 内整合のため必要に応じ `pub(crate)` へ調整（外部公開範囲は拡大しない）。
- `WsBroadcaster` / `WsSender` / `WsReceiver` / `StreamOutbound`（private）：定義不変。
  定数 `STREAM_DELTA_QUEUE_LIMIT` = 1024 / `STREAM_DELTA_QUEUE_BYTE_LIMIT` = 512KiB は変更しない。
- `AgentStateSync`（protocol）への変換ロジックは不変。

## 処理フロー

移設後も実行フローは現状と同一である。

1. `server_control::start_server_core`（middleware）が config 読込・TLS cert 確保（remote_access usecase）・
   `WsServerState` 構築・`http_upgrade::start_ws_server` 呼び出しを行い、`WsServerHandle` を更新、
   `server-status-changed` を emit する。
2. `http_upgrade`（middleware）が TCP accept → TLS → HTTP upgrade を処理し、`session::handle_ws_session` へ委譲。
3. `session`（middleware）が rate limit check → 同時接続制限 → HMAC handshake（`auth` 使用）→ broadcaster
   sender 登録 → forward task spawn → PTY replay → 受信ループを実行。受信ループで decode 後
   `handler::route_message` を呼び、戻り message を broadcaster へ返す。
4. `handler::route_message`（controller）が ResyncStream を resync usecase へ dispatch し、snapshot を
   broadcaster へ enqueue、または Error message を返す。
5. agent status 変更時は各 caller が `presenter::agent_status::emit_agent_status_changes` を呼び、
   Tauri emit と broadcaster push を行う。

stream buffering / collapse / drain / forward の動作、queue・byte 上限の挙動、disconnect 時の queue clear は
`WsBroadcaster` 実装をそのまま移すため不変。

## エラー処理

- 既存のエラー string・エラーコード（`INVALID_MESSAGE` / `STREAM_NOT_FOUND` / `STREAM_RESYNC_FAILED` 等）、
  `AuthResult` の失敗 message、security block の拒否メッセージは一切変更しない。
- module ごとの専用 error type を新設する必要はない（現状 `Result<_, String>` ベースを踏襲）。本 ISSUE は
  エラー設計の改善を含まない（スコープ外）。

## テスト方針

既存 test は対象 module へ併設のまま追従させ、期待値・アサーション内容は変更しない。

| test 群（現在） | 移設先 |
|---|---|
| `ws_server/auth.rs` の 7 test | `infrastructure/middleware/auth.rs` |
| `ws_server/rate_limit.rs` の 7 test | `infrastructure/middleware/rate_limit.rs` |
| `ws_server/http.rs` の 3 test（security block） | `infrastructure/middleware/http_upgrade.rs` |
| `ws_server/session.rs` の 2 test（PTY replay） | `infrastructure/middleware/session.rs` |
| `ws_server/routing.rs` の 4 test | `adaptor/controller/handler/mod.rs` |
| `ws_server/mod.rs` の 3 test（deserialize_message） | `adaptor/controller/handler/mod.rs`（dispatch 入力検証として併置） |
| `ws_bridge.rs` の 15 test | `adaptor/gateway/shared/ws_broadcaster.rs` |

特記事項:

- **routing test の setup 変更**: `route_message` のシグネチャ変更に伴い、routing test は `WsServerState`
  （infra 型）を構築せず、`broadcaster` と mock `AgentStreamResyncReadModel` を直接渡す形へ setup を変更する。
  アサーション（`INVALID_MESSAGE` / `STREAM_NOT_FOUND` / snapshot enqueue / `STREAM_RESYNC_FAILED`）は不変。
  これに伴い routing test 専用の `MockConfigRepository` / `EmptyReplayReader` は不要となり削除する
  （これらは期待値ではなく setup 補助である）。
- **session test**: `handle_ws_authenticated` を直接呼ぶ test は、`route_message` 結合解消後も session が
  内部で broadcaster / read model を渡すため、test setup（`WsServerState::new` 構築）は現状維持で通る。
- **依存ガードテスト更新**: `usecase/workflow/mod.rs` の `outer_modules` リストから、削除されるルート module 名
  `"ws_bridge"` / `"ws_server"` / `"agent_status_events"` を除去する。これら 3 名は実体が消えるため
  stale 参照となる。usecase/workflow が transport へ依存しないという assertion は、残る `"adaptor"` /
  `"infrastructure"` エントリで引き続き担保される（assertion の意味は不変、stale 名の除去のみ）。
- 品質ゲート: `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` を通す。

## リスクと代替案

- **リスク: import 経路の取りこぼし**。`WsBroadcaster` は 7 ファイル以上、`emit_agent_status_changes` は
  3 ファイルから参照される。grep ベースで全参照を洗い、`crate::ws_bridge::WsBroadcaster` →
  `crate::adaptor::gateway::shared::ws_broadcaster::WsBroadcaster`、`crate::agent_status_events::*` →
  `crate::adaptor::presenter::agent_status::*` を網羅置換する。コンパイルエラーで検出可能。
- **リスク: `route_message` シグネチャ変更による回帰**。内部ロジックは不変で参照元を引数へ置換するだけ。
  routing test 4 件のアサーションが維持されることで担保する。
- **代替案 A（route_message を `WsServerState` のまま handler へ移す）**: handler が infrastructure 型を
  import することになり層方向が乱れる。採用しない。
- **代替案 B（`WsBroadcaster` を GATEWAY.md に厳密準拠し domain trait + infrastructure 送信実装へ分離）**:
  GATEWAY.md は「送信実装は infrastructure、gateway は trait 実装」を推奨するが、これは新 trait 導入と
  呼び出し側書き換えを伴い「配置のみ移す・振る舞い維持」の本 ISSUE スコープを超える。ISSUE は
  「outbound broadcaster → adaptor/gateway」を binding 要件とするため、`WsBroadcaster` を as-is で
  `adaptor/gateway/shared/` へ移す方針を採る。trait 抽出は別 ISSUE 候補として残す。
- **代替案 C（`emit_agent_status_changes` を presenter（純変換）と gateway/controller（送出 wiring）へ分割）**:
  純度は上がるが、6 箇所の caller 書き換えと送出責務の所在判断（gateway か controller か）を伴い churn と
  回帰リスクが増す。39 行の関数を whole-move する方が振る舞い維持に資するため、presenter への一括移設を採る。
  Tauri emit / broadcaster push は payload 整形に続く送出として presenter helper 内に保持する。

## 仮定

- requirements.md / behavior.md の仮定（wire 互換維持、#1130 完了、broadcaster 定数不変、`infrastructure/middleware/`
  新規作成）を引き継ぐ。
- `start_server_core` / `stop_server_core` は現状 `#[allow(dead_code)]` で呼び出し元が存在しない。本 ISSUE では
  配置のみ移し、`#[allow(dead_code)]` を維持する（command 登録への wiring は本 ISSUE スコープ外）。
- handler は現状 ResyncStream 1 種のみを扱うため `handler/mod.rs` 単一ファイルに集約する。ドメイン別
  submodule（`handler/<domain>/<usecase>.rs`）への分割は message 種別が増えた時点で行う。
- `WsBroadcaster` を `adaptor/gateway/shared/` へ as-is 移設し、domain trait 抽出は行わない（スコープ外）。
- `emit_agent_status_changes` は presenter へ whole-move し、純変換と送出 wiring の分割は行わない。
- 依存ガードテストの forbidden-list 更新は、削除 module 名の除去であり assertion の意味を変えない。

## Open Questions

なし。
