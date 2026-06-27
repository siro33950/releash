# Requirements

## Type

実装 ISSUE（リファクタリング）。親 ISSUE ではない。

関連: #1130 / Blocks #1131 / Related #878 (final sweep)

マイルストーン: [12] クリーンアーキテクチャ移行。

## 背景と目的

現状、protocol 境界の定義が `src-tauri/src/protocol/` と `src-tauri/src/adaptor/protocol/` の 2 箇所に併存している。

- `src-tauri/src/protocol/` — `mod.rs` / `agent.rs` / `auth.rs` / `branch.rs` / `error.rs` / `worktree.rs`。`WsMessage` 列挙、`serialize_message` / `deserialize_message` helper、各種 WebSocket payload type がここにある。
- `src-tauri/src/adaptor/protocol/` — `mod.rs` / `code.rs` / `mention.rs` / `pty.rs` / `workflow.rs`。clean architecture 移行で新設された protocol 境界。

この二重化は「protocol 境界の所有者が 1 つに定まっていない」状態であり、clean architecture の層構造（adaptor が wire shape を所有する）に反する。本 ISSUE はこの二重化を解消し、protocol 境界を `src-tauri/src/adaptor/protocol/` に一本化することを目的とする。

本 ISSUE は構造（モジュール配置と import 経路）の統一を担当するものであり、WebSocket の対外契約（message 名・payload JSON shape）や domain / usecase の振る舞いは変更しない。

## スコープ

- `src-tauri/src/protocol/` 配下の型・helper（`WsMessage` / `serialize_message` / `deserialize_message` / agent / auth / branch / error / worktree payload type）を `src-tauri/src/adaptor/protocol/` 配下へ移設する。
- `lib.rs`（および該当 module 宣言箇所）から `mod protocol`（ルート直下）を削除する。
- production code 全体の `crate::protocol::*` import を `crate::adaptor::protocol::*`（または adaptor 内の相対経路）へ置き換える。
- 既存の protocol roundtrip / serialize-deserialize test を新 module 側へ移設し、維持する。
- ISSUE 「責務範囲」に沿って、移設先での配置を以下の層責務に従わせる:
  - `adaptor/protocol/` — WebSocket envelope / message payload type、wire shape である Tauri command input type、serialize / deserialize helper。
  - `adaptor/presenter/` — domain / usecase read model から protocol response shape への mapping。
  - `adaptor/controller/` — protocol request shape から usecase input への mapping。

## 非スコープ

- WebSocket message 名や payload JSON contract の変更。シリアライズ結果の互換性は維持する。
- `ws_server/` の session / auth / routing / rate-limit の構造変更（#1131 の担当）。
- `ws_bridge.rs` の buffering / broadcast lifecycle の変更（#1131 の担当）。
- domain behavior や command 実装ロジックそのものの移動・変更。
- フロントエンド（TypeScript）側の型定義・通信コードの変更。wire contract が不変であるため frontend への影響はないものとする。
- `adaptor/presenter/` / `adaptor/controller/` への mapping ロジックの新規切り出し・再設計（責務の最終形を示す指針ではあるが、本 ISSUE では二重化解消に必要な範囲に限る。詳細は design.md）。

## 要求事項

### protocol 境界の一本化

- protocol 境界の型・helper は `src-tauri/src/adaptor/protocol/` 配下にのみ存在すること。
- `src-tauri/src/protocol/` ディレクトリが削除されていること。
- `lib.rs` にルート直下の `mod protocol` 宣言が残っていないこと。

### import 経路の統一

- production code に `crate::protocol`（ルート直下 protocol への参照）が残っていないこと。
- 移設対象の型・helper を参照する全モジュールが、新しい `adaptor/protocol/` 経路を参照していること。

### 主要シンボルの配置

- `WsMessage` が `adaptor/protocol/` 配下にあること。
- `serialize_message` が `adaptor/protocol/` 配下にあること。
- `deserialize_message` が `adaptor/protocol/` 配下にあること。

### 層責務の維持

- domain / usecase module が `adaptor::protocol` を import していないこと（依存方向: adaptor → usecase → domain を逆転させない）。

### 対外契約の不変

- WebSocket message 名が変わらないこと。
- payload の JSON shape（serialize 結果）が変わらないこと。

### テストの維持

- 既存の protocol roundtrip / serialize-deserialize test が新 module 側で維持され、通過すること。

### 品質ゲート

- `cargo fmt --check` が通ること。
- `cargo clippy -- -D warnings` が通ること。
- `cargo test` が通ること。

## 受け入れ基準の概要

ISSUE の「完了条件」をそのまま受け入れ基準とする。

- `src-tauri/src/protocol/` が削除されている。
- `lib.rs` に `mod protocol` が残っていない。
- `rg 'crate::protocol' src-tauri/src --glob '*.rs'` が production reference を返さない。
- `WsMessage` / `serialize_message` / `deserialize_message` が `adaptor/protocol/` 配下にある。
- domain / usecase module が `adaptor::protocol` を import していない。
- 既存 protocol roundtrip test が新 module 側で維持されている。
- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る。

## 仮定

- 移設は「型・helper の物理的な移動と import 経路の更新」を主とし、wire 上のシリアライズ結果は完全に維持する。message 名・payload JSON shape を一切変えないことで、frontend およびネットワーク互換性に影響を与えない。
- `protocol/` 配下の各ファイル（agent / auth / branch / error / worktree / mod）を `adaptor/protocol/` 配下のどのファイル構成に落とし込むか（既存の `adaptor/protocol/{code,mention,pty,workflow,mod}.rs` への統合方針、ファイル分割粒度）の具体は本要求では確定させず、design.md で決定する。
- `adaptor/presenter/` / `adaptor/controller/` への mapping は ISSUE が示す将来の責務境界であり、本 ISSUE では二重化解消（型の一本化と import 統一）の達成に必要な範囲でのみ触れる。mapping ロジックの全面的な再配置は別 ISSUE / 後続作業として扱う。
- `crate::protocol` を参照している production module（ws_server / ws_bridge / agent_status_events / adaptor 配下 gateway・controller / infrastructure 配下 agent_session runtime 等）は、振る舞いを変えずに import 経路のみ更新する。
- 「production reference を返さない」は、test code 内の参照や doc comment ではなく、ビルド対象となる production path の `crate::protocol` 参照を指すものとする。

## Open Questions

なし。
