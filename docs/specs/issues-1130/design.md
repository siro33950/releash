# Design

本書は #1130「protocol 境界の `adaptor/protocol/` への一本化」の実装設計である。
`requirements.md` / `behavior.md` を前提とし、移設対象ファイルの具体的な配置・分割粒度、
import 経路の更新範囲、対外契約不変の担保方法、テスト方針を確定する。

## 概要

protocol 境界の型・helper が `src-tauri/src/protocol/`（ルート直下）と
`src-tauri/src/adaptor/protocol/` の 2 箇所に併存している二重化を解消し、
protocol 境界を `adaptor/protocol/` に一本化する。

本 ISSUE はリファクタリングであり、対外契約（WebSocket message 名・payload JSON shape）と
domain / usecase の振る舞いは変更しない。型・helper の物理移動と import 経路の更新に限る。

### 現状の構造（調査結果）

- ルート直下 `protocol/`:
  - `mod.rs` — `WsMessage` 列挙（`#[serde(tag="type", content="payload")]`）、
    `serialize_message` / `deserialize_message`、roundtrip 系 test。
  - `agent.rs` / `auth.rs` / `branch.rs` / `error.rs` / `worktree.rs` — 各 payload type。
  - `lib.rs:18` に `mod protocol;`（ルート直下宣言）がある。
- `adaptor/protocol/`:
  - `mod.rs` — module 宣言のみ（`pub(crate) mod code/mention/pty/workflow;`）。
  - `code.rs` / `mention.rs` / `pty.rs` / `workflow.rs` — Tauri command 入力型・WebSocket payload type。
  - `adaptor/mod.rs:5` に `pub(crate) mod protocol;` がある。

重要な現状の事実として、**ルート `protocol/mod.rs` の `WsMessage` は既に
`crate::adaptor::protocol::pty`（`PtyOutputMsg` 等）と `crate::adaptor::protocol::workflow`
（`WorkflowStateSync`）を import している**。すなわち WsMessage の集約点だけが
ルート直下に残り、payload の一部は既に adaptor 側へ移っている「移行途中」の状態である。
本 ISSUE はこの集約点と残りの payload type を adaptor 側へ寄せて一本化する。

- production code の `crate::protocol` 参照: 12 ファイル / 34 箇所（test mod 内参照を含む）。
  - `ws_server/{mod,routing,session}.rs`、`ws_bridge.rs`、`agent_status_events.rs`、
    `adaptor/gateway/{pty_session/backend_impl, repository/state, workflow/state_notification_gateway}.rs`、
    `adaptor/controller/command/agent_session/session.rs`、
    `infrastructure/agent_session/runtime/bridge_common/{sdk_message,session_persistence,stream_emit}.rs`。
- domain / usecase 層には `crate::protocol` 参照は存在しない（依存方向は既に健全）。

## 変更対象

### 移動するファイル（root `protocol/` → `adaptor/protocol/`）

| 移動元 | 移動先 | 内容 |
|---|---|---|
| `protocol/agent.rs` | `adaptor/protocol/agent.rs` | agent 系 payload type と `From` 実装 |
| `protocol/auth.rs` | `adaptor/protocol/auth.rs` | auth 系 payload type |
| `protocol/branch.rs` | `adaptor/protocol/branch.rs` | branch 系 payload type と `From` 実装 |
| `protocol/error.rs` | `adaptor/protocol/error.rs` | `ErrorMsg` |
| `protocol/worktree.rs` | `adaptor/protocol/worktree.rs` | worktree 系 payload type |
| `protocol/mod.rs` の内容 | `adaptor/protocol/mod.rs` へ統合 | `WsMessage` / `serialize_message` / `deserialize_message` / roundtrip test |

移動先ファイル名はいずれも `adaptor/protocol/` 配下で既存ファイル（`code/mention/pty/workflow`）と
衝突しないため、ファイル分割粒度はルート側の現状をそのまま 1:1 で踏襲する。
**ファイルを統廃合せず 1:1 で移すことで diff を最小化し、対外契約の不変を確認しやすくする。**

### 削除する宣言・ディレクトリ

- `src-tauri/src/protocol/` ディレクトリ全体。
- `lib.rs:18` の `mod protocol;`。

### 更新するファイル（import 経路のみ）

- 上記「現状の構造」で列挙した 12 production ファイルの `crate::protocol::*` を
  `crate::adaptor::protocol::*` へ置換する。
- test mod 内（`ws_server/routing.rs` / `ws_server/session.rs` / `ws_server/mod.rs` の
  `#[cfg(test)]` ブロック）の `crate::protocol::*` 参照も同様に置換する。
  受け入れ基準は production reference のみを対象とするが、ルート `protocol` が消える以上
  test 参照も解決不能になるため、ビルドを通すために必須の更新である。

## アーキテクチャと責務分割

移設後の `adaptor/protocol/` は、clean architecture における「adaptor が wire shape を所有する」
原則に従い、WebSocket envelope / message payload type、wire shape の Tauri command input type、
serialize / deserialize helper の唯一の所有者となる。

```
adaptor/protocol/
├── mod.rs        WsMessage 列挙 + serialize_message / deserialize_message + roundtrip test
├── agent.rs      agent 系 payload type（移設）
├── auth.rs       auth 系 payload type（移設）
├── branch.rs     branch 系 payload type（移設）
├── error.rs      ErrorMsg（移設）
├── worktree.rs   worktree 系 payload type（移設）
├── code.rs       既存
├── mention.rs    既存
├── pty.rs        既存
└── workflow.rs   既存
```

### mapping（`From` 実装）の扱い

`protocol/agent.rs` と `protocol/branch.rs` には domain / usecase 型から protocol payload への
`From` 実装が含まれる（`From<usecase::…::AgentState>`、`From<MessagePart>`、`From<BranchCardDto>` 等）。

層責務の最終形では、これらの mapping は `adaptor/presenter/` に置かれるべきものである。
ただし requirements §非スコープ／§仮定の通り、**mapping ロジックの presenter への切り出し・再設計は本 ISSUE のスコープ外**である。
したがって本 ISSUE では `From` 実装を payload type と同じファイルに置いたまま `adaptor/protocol/` へ移設する。
presenter への移動は後続作業（#878 final sweep 等）に委ねる。この方針は本書の「リスクと代替案」で根拠を示す。

### 依存方向の維持

移設後も domain / usecase 層は `crate::adaptor::protocol` を import しない。
`From` 実装は protocol 型側（adaptor）が usecase / domain 型を参照する形であり、
依存方向（adaptor → usecase → domain）を逆転させない。これは現状と同一の依存形である。

## データモデルまたは型

型定義そのものは一切変更しない。`WsMessage` 列挙のバリアント、各 payload struct のフィールド、
serde 属性（`#[serde(tag="type", content="payload")]`、`#[serde(rename=...)]`、
`#[serde(rename_all=...)]`、`Option` フィールドの省略挙動）はすべて現状のまま移設する。

`adaptor/protocol/mod.rs` の最終形（統合後）:

```rust
//! 外部入口（Tauri コマンド引数・WebSocket メッセージ）のメッセージ型。
//! （既存 doc comment を維持）

pub(crate) mod agent;
pub(crate) mod auth;
pub(crate) mod branch;
pub(crate) mod code;
pub(crate) mod error;
pub(crate) mod mention;
pub(crate) mod pty;
pub(crate) mod worktree;
pub(crate) mod workflow;

pub(crate) use agent::*;
pub(crate) use auth::*;
pub(crate) use branch::*;
pub(crate) use error::*;
pub(crate) use worktree::*;

use crate::adaptor::protocol::pty::{PtyEvictedMsg, PtyExitMsg, PtyOutputMsg};
use crate::adaptor::protocol::workflow::WorkflowStateSync;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
#[allow(clippy::large_enum_variant)]
pub enum WsMessage { /* 現状のバリアントをそのまま */ }

pub fn serialize_message(msg: &WsMessage) -> Result<String, String> { /* 現状のまま */ }
pub fn deserialize_message(json: &str) -> Result<WsMessage, String> { /* 現状のまま */ }

#[cfg(test)]
mod tests { /* 現状の roundtrip / serialize-deserialize test をそのまま */ }
```

### 可視性（visibility）の方針

ルート `protocol/mod.rs` は `pub use agent::*` 等で公開していたが、protocol 型は crate 内部からのみ
参照される（外部公開しない）。移設先では既存 `adaptor/protocol/` の慣習に合わせ、
`pub(crate) mod` / `pub(crate) use` に統一する。これにより protocol AST が
crate 外へ漏れない境界（spec [05] の内部 AST 非公開境界と整合）を維持する。

> 仮定: `pub(crate)` への統一でビルドが通る（外部 crate からの参照は存在しない）。
> もし `lib.rs` の Tauri command 登録等で外部可視性が必要な箇所があれば、その型のみ `pub` を維持する。

## 処理フロー

リファクタリングのため runtime の処理フローは不変。作業手順としてのフローは以下:

1. `adaptor/protocol/mod.rs` を統合形へ更新（`mod`/`use` 宣言追加、`WsMessage` /
   helper / test の移植）。
2. `protocol/{agent,auth,branch,error,worktree}.rs` を `adaptor/protocol/` 配下へ移動。
   移動に伴い、ファイル内の `crate::adaptor::protocol::…` 自己参照や `super::` 経路を必要に応じて調整する
   （ルート時代に `crate::adaptor::protocol::pty` 等を絶対経路で参照していた箇所はそのまま有効）。
3. `lib.rs:18` の `mod protocol;` を削除し、`protocol/` ディレクトリを削除する。
4. 12 production ファイル + 関連 test mod の `crate::protocol::*` を
   `crate::adaptor::protocol::*` へ一括置換する。
5. 品質ゲート（`cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`）を実行する。

## エラー処理

- `serialize_message` / `deserialize_message` のエラー文言・`Result<_, String>` 型は変更しない
  （「シリアライズ失敗: …」「デシリアライズ失敗: …」をそのまま維持）。
- 未知 message type が deserialize でエラーになる挙動（serde の untagged 失敗）を維持する。
  behavior.md の「未知の message type は移設後も拒否される」を担保する。
- 本作業で新たな error type の追加・変更は行わない。

## テスト方針

- ルート `protocol/mod.rs` の `#[cfg(test)] mod tests`（`serialize_auth_challenge` /
  `roundtrip_auth_result_with_message` / `auth_result_omits_none_message` / `roundtrip_error` /
  `deserialize_unknown_type_fails` / `all_variants_roundtrip`）を、内容を変えずに
  `adaptor/protocol/mod.rs` 側へ移設する。
- `protocol/agent.rs` 内の test（3 箇所）も `adaptor/protocol/agent.rs` へファイルごと移設される。
- behavior.md の各 Rule が以下の test で担保されることを確認する:
  - 対外契約の不変（variant ごとの serialize 結果一致 / deserialize roundtrip / None 省略 /
    未知 type 拒否） → 既存 `all_variants_roundtrip`・`auth_result_omits_none_message`・
    `deserialize_unknown_type_fails` 等が移設後も通過することで担保。
  - 構造的不変条件（ディレクトリ削除 / `crate::protocol` 参照ゼロ / シンボル配置） →
    test ではなくビルド成立と `rg` 検査で担保（実装完了時に手動確認）。
- 新規 test は追加しない（リファクタリングのため）。既存 test の移設・通過のみを成果とする。
- 検証コマンド（`src-tauri/` で実行）:
  - `cargo fmt --check`
  - `cargo clippy -- -D warnings`
  - `cargo test`
  - `rg 'crate::protocol' src-tauri/src --glob '*.rs'` が production path に参照を返さないこと。

## リスクと代替案

### リスク 1: import 一括置換による参照漏れ・誤置換

`crate::protocol::*` を `crate::adaptor::protocol::*` へ機械置換する際、文字列一致で
`crate::adaptor::protocol` のような既に正しい経路を二重置換しないよう注意する。
対策: 置換は `crate::protocol`（直後が `::` か行末）に限定し、置換後に
`rg 'crate::adaptor::adaptor'` 等で誤置換が無いことを確認、最終的に `cargo build` / `cargo test` で担保する。

### リスク 2: serde 出力の非互換（対外契約破壊）

最大の禁止事項。型・属性を 1:1 で移すことで shape を保つ。`all_variants_roundtrip` が
serialize→deserialize→serialize の一致を検証するため、属性の取りこぼしは test で検出される。
ただし「移設前後の JSON 一致」までは既存 test が直接比較しないため、
属性をファイル単位でそのまま移動する（手で書き換えない）方針でリスクを最小化する。

### 代替案 A: mapping を本 ISSUE で presenter へ切り出す

`From` 実装を `adaptor/presenter/` へ移すと層責務はより理想形に近づくが、
requirements が明確に非スコープと定めており、二重化解消という本 ISSUE の目的を超えて
diff と検証範囲を拡大させる。採用しない（後続 ISSUE で対応）。

### 代替案 B: ファイルを統合して粒度を変える（例: agent/auth/branch を 1 ファイルに）

統合は diff を増やし、移設の等価性確認を難しくする。1:1 移設を採用する。

## 仮定

- 移設は型・helper の物理移動と import 経路更新に限り、serde シリアライズ結果を完全に維持する。
  message 名・payload JSON shape を変えないため frontend / ネットワーク互換性に影響しない。
- ファイル分割粒度はルート `protocol/` の現状を 1:1 で踏襲する（統廃合しない）。
- 移設先の可視性は `pub(crate)` に統一する。外部 crate からの参照は存在しない前提。
  必要な箇所のみ `pub` を残す。
- `From` 実装（mapping）は payload type と同居したまま移設し、presenter への切り出しは
  本 ISSUE のスコープ外（後続作業）とする。
- 「production reference を返さない」は、ビルド対象 production path の `crate::protocol`
  （ルート直下）参照を指す。doc comment や移設後の `adaptor::protocol` 経路は対象外。
- test mod 内の `crate::protocol` 参照も、ビルド成立のため本作業で更新する
  （受け入れ基準の対象外だが必須の付随更新）。

## Open Questions

なし。
