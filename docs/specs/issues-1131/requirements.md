# Requirements

## Type

実装 ISSUE（リファクタリング）。親 ISSUE ではない。

関連: Depends on #1130（完了済み） / Blocks #878（final sweep） / Related #1133（root glue cleanup）

マイルストーン: [12] クリーンアーキテクチャ移行。

## 背景と目的

clean architecture への段階移行において、protocol 境界の一本化（#1130）は完了し、protocol 型・helper は `src-tauri/src/adaptor/protocol/` 配下に集約済みである。一方で、WebSocket server 本体と app/WS の broadcast bridge は、依然としてルート直下に layer 未整理のまま残っている。

- `src-tauri/src/ws_server/` — `mod.rs` / `auth.rs` / `commands.rs` / `http.rs` / `rate_limit.rs` / `routing.rs` / `session.rs`。WebSocket server の起動・bind・TLS plumbing、HMAC auth、rate limit、HTTP upgrade、message routing、session lifecycle がここにある。
- `src-tauri/src/ws_bridge.rs` — `WsBroadcaster`。WS sender 登録、push（`try_send` / forward）、agent stream の delta/snapshot buffering（queue 上限・byte 上限・slow consumer 向け snapshot 折りたたみ）と drain 通知を担う。
- `src-tauri/src/agent_status_events.rs` — usecase の `AgentStatusChanges` を Tauri emit event と WS push（`WsMessage::AgentStateSync`）へ変換する。usecase status change を transport push へ変換しているため本 ISSUE で扱う。

これらは「transport 境界（adaptor / infrastructure）に属するコードがルート直下に置かれ、層責務の所有者が定まっていない」状態であり、clean architecture の層構造（adaptor が wire 境界を所有し、infrastructure が純粋 transport concern を持つ）に反する。本 ISSUE はこの未整理を解消し、WebSocket server / bridge / status event 変換を ISSUE が示す層責務へ配置することを目的とする。

本 ISSUE は構造（モジュール配置と import 経路）の整理を担当するものであり、WebSocket の対外契約（message 名・payload JSON shape）や domain / usecase の振る舞いは変更しない。既存の auth、reconnect、resync、PTY replay、stream buffering、push notification の振る舞いを維持する。

## スコープ

- 対象コードを ISSUE の「責務範囲」に従い、以下の層へ移設する:
  - `adaptor/controller/handler/` — WebSocket routing / request handler entrypoint。protocol message を usecase call へ変換する責務（現 `ws_server/routing.rs` / `ws_server/session.rs` / `ws_server/commands.rs` のうち request dispatch に相当する部分）。
  - `infrastructure/middleware/` — HMAC auth（現 `ws_server/auth.rs`）、rate limit（現 `ws_server/rate_limit.rs`）、HTTP upgrade helper（現 `ws_server/http.rs`）、TLS / server bind plumbing のうち純粋 transport concern（現 `ws_server/mod.rs` の server state / 起動・bind 部分）。
  - `adaptor/gateway/...` — outbound broadcaster 実装（現 `ws_bridge.rs` の `WsBroadcaster`）、push / sync notifier adapter、PTY / repository / workflow / agent status の push gateway。
  - `adaptor/presenter/...` — usecase / domain state から protocol push payload への変換（現 `agent_status_events.rs` の `AgentStatusChanges` → `WsMessage` / Tauri emit payload 変換）。
- `lib.rs`（および該当 module 宣言箇所）から `mod ws_server` / `mod ws_bridge` / `mod agent_status_events` を削除し、移設先 module への参照へ置き換える。
- 上記移設対象を参照する production code の import 経路を、新しい module 経路へ更新する。
- 既存の WebSocket 関連 test（auth success/failure、invalid message、rate limit、resync stream、buffered replay、broadcaster drop / buffer limit、HTTP upgrade、routing 等）を移設先 module 側へ移し、維持する。

## 非スコープ

- protocol type の移設・再配置（#1130 で完了済み。本 ISSUE では touch しない）。
- WebSocket message 名や payload JSON contract の変更。シリアライズ結果の互換性は維持する。
- remote / mobile の product scope（接続モード・リモートアクセス設計）の再設計。
- menu / tray / native drop / path aliases などの root glue cleanup（#1133 の担当）。
- dead code sweep（#878 の担当）。
- domain / usecase の振る舞いや command 実装ロジックそのものの変更。
- フロントエンド（TypeScript）側の型定義・通信コードの変更。wire contract が不変であるため frontend への影響はないものとする。
- broadcaster の buffering / broadcast lifecycle のアルゴリズム変更（queue 上限・byte 上限・snapshot 折りたたみ等の振る舞いは現状維持。配置のみ移す）。

## 要求事項

### ルート直下 module の解消

- `src-tauri/src/ws_server/` ディレクトリが削除されていること。
- `src-tauri/src/ws_bridge.rs` が削除されていること。
- `src-tauri/src/agent_status_events.rs` が削除されるか、明確な adaptor / presenter / gateway module へ移動していること。
- `lib.rs` に `mod ws_server` / `mod ws_bridge` / `mod agent_status_events` 宣言が残っていないこと。

### 層責務に沿った配置

- WebSocket routing / request handler entrypoint（protocol → usecase 変換）が `adaptor/controller/handler/` 配下にあること。
- HMAC auth / rate limit / HTTP upgrade helper / TLS・server plumbing のうち純粋 transport concern が `infrastructure/middleware/` 配下にあること。
- outbound broadcaster 実装および push / sync notifier adapter が `adaptor/gateway/` 配下にあること。
- usecase / domain state から protocol push payload への変換が `adaptor/presenter/` 配下にあること。

### 依存方向の維持

- domain / usecase module が adaptor / infrastructure を import していないこと（依存方向: adaptor / infrastructure → usecase → domain を逆転させない）。

### 振る舞いの維持

- 既存の WebSocket auth、reconnect、resync、PTY replay、stream buffering、push notification の振る舞いが維持されていること。
- WebSocket message 名が変わらないこと。
- payload の JSON shape（serialize 結果）が変わらないこと。

### テストの維持

- auth success / failure、invalid message、resync stream、buffered replay、broadcaster drop / buffer limit の test が（移設先 module 側で）存在し、通過すること。
- 移設前に存在した rate limit / HTTP upgrade / routing 等の test が失われていないこと。

### 品質ゲート

- `cargo fmt --check` が通ること。
- `cargo clippy -- -D warnings` が通ること。
- `cargo test` が通ること。

## 受け入れ基準の概要

ISSUE の「完了条件」をそのまま受け入れ基準とする。

- `src-tauri/src/ws_server/` が削除されている。
- `src-tauri/src/ws_bridge.rs` が削除されている。
- `src-tauri/src/agent_status_events.rs` が削除されるか、明確な adaptor / presenter / gateway module へ移動している。
- `lib.rs` に `mod ws_server` / `mod ws_bridge` / `mod agent_status_events` が残っていない。
- 既存の WebSocket auth、reconnect、resync、PTY replay、stream buffering、push notification behavior が維持されている。
- auth success/failure、invalid message、resync stream、buffered replay、broadcaster drop/buffer limit の test がある。
- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る。

## 仮定

- 本 ISSUE は「コードの物理的な移動と import 経路の更新」を主とし、wire 上のシリアライズ結果（message 名・payload JSON shape）と既存の transport 振る舞い（auth / reconnect / resync / PTY replay / stream buffering / push notification）を完全に維持する。frontend およびネットワーク互換性に影響を与えない。
- #1130 が完了済みであり、protocol 型・helper は `src-tauri/src/adaptor/protocol/` 配下にある前提とする。本 ISSUE は protocol module を touch しない。
- `ws_server/` 配下の各ファイル（auth / commands / http / rate_limit / routing / session / mod）を、ISSUE が示す 4 つの層（controller/handler, infrastructure/middleware, gateway, presenter）のどの module・ファイル構成へ落とし込むか（分割粒度、`WsServerState` / `WsServerHandle` の配置、起動・bind・shutdown plumbing の所属）の具体は本要求では確定させず、design.md で決定する。
- `agent_status_events.rs` は「usecase status change → push payload 変換」という性質から presenter 寄りだが、Tauri emit と broadcaster push の両方を起動する wiring 的側面も持つ。最終的な配置（presenter への純変換切り出し + wiring の controller 側残置、など）は design.md で決定する。本要求では「削除または明確な adaptor/presenter/gateway module への移動」を満たすことのみを要件とする。
- `infrastructure/middleware/` ディレクトリは現状存在しないため、本 ISSUE で新規作成する。
- 既存 test は対象 module に併設された `#[cfg(test)] mod tests` 形式（auth 7、http 3、rate_limit 7、routing 4、session 2、ws_server/mod 3、ws_bridge 15 等）であり、これらは移設先 module へ追従させる。test の期待値・アサーション内容は変更しない。
- broadcaster の queue 上限（`STREAM_DELTA_QUEUE_LIMIT` = 1024）・byte 上限（`STREAM_DELTA_QUEUE_BYTE_LIMIT` = 512KiB）・snapshot 折りたたみアルゴリズムは振る舞い維持対象であり、定数・ロジックを変更しない。

## Open Questions

なし。
