# Design

本書は `requirements.md` / `behavior.md` を受けて、`bridge_common.rs`（21,225 行）の責務別分割の実装方針を確定する。本 Issue は内部構造リファクタリングであり、observable behavior は変えない（`behavior.md` 参照）。

## 概要

`src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs` を、単一の `.rs` ファイルから `bridge_common/` ディレクトリ（`mod.rs` + 責務別サブモジュール群）へ分割する。

分割の基本戦略は次の 2 点に集約する。

1. **外部互換 symbol の明示 facade**: `bridge_common/mod.rs` はサブモジュール宣言と、外部参照元（`runtime/mod.rs`・`codex.rs`・`claude.rs`）が互換維持に必要とする symbol の明示 `pub(crate) use sub::{...};` のみに限定する。`bridge_common::Foo` の参照パスは必要な公開 API だけ維持し、内部 helper は各責務 module 内または `pub(super)` / private / test-only に閉じる。
2. **production と test の同時移動**: 各 production 要素を責務別サブモジュールへ移すとき、その要素を検証する `#[cfg(test)]` テストも同じサブモジュール内へ移す。テストの期待値は変更しない（R2 / R5）。

requirements R1 が要求する 5 責務（runtime/process registry・stream emit・session persistence・permission・recovery）を必須サブモジュールとして立てる。5 責務に自然に属さない既存 production コード（session lifecycle・model 選択・skills・external agent・SDK message accumulation・turn event log・共有 helper）は、補助サブモジュールとして併設する（**仮定 A-split**、後述）。

## 変更対象

### 新設（`bridge_common/` ディレクトリ）

| ファイル | 責務 | requirements 対応 |
|---|---|---|
| `bridge_common/mod.rs` | サブモジュール宣言と、外部互換に必要な symbol だけの明示 re-export。分割前の公開境界を無差別に再公開せず、内部 helper は責務 module に閉じる。 | R1 / R4 / A6 |
| `bridge_common/process_registry.rs` | runtime 状態型（`AgentProcess`・`BridgeState`・`TurnPhase`・`PendingMessage`・`AgentProcessMap`）と per-session process map 管理、liveness/turn-state メソッド。 | R1-1 |
| `bridge_common/stream_emit.rs` | `streaming_parts` payload 生成、Tauri event / WS 両チャネルへの集約 emit、flush 閾値・interval・byte cap、streaming timer。permission / lifecycle / recovery / SDK message 分類の状態遷移は保持しない。 | R1-2 |
| `bridge_common/session_persistence.rs` | streaming parts / post-turn base parts の persist・load、turn event log の永続化連携、agent session_id・context carry の persist/load。 | R1-3 |
| `bridge_common/permission.rs` | permission mode 設定、permission 要求の state 遷移、permission 応答、resolution 記録。 | R1-4 |
| `bridge_common/recovery.rs` | PID ファイル管理・orphan cleanup、process 死活検知・再 spawn、stale timeout watchdog、resume/context-restore mismatch 時の requeue。 | R1-5 |
| `bridge_common/session_lifecycle.rs` | session の start/close/init、turn 開始、message 送信、interrupt/cancel、`get_session`（read 系）。read 経路と write 経路を本ファイル内で関数単位に分離（R4）。 | 補助（A-split） |
| `bridge_common/turn_event_log.rs` | turn event log の開始・durable parts 記録・projected state。 | 補助（A-split） |
| `bridge_common/sdk_message.rs` | SDK message router（`handle_sdk_message`）と part accumulation（text/thinking/tool_use/tool_result/permission/error/task_status/compaction）。 | 補助（A-split） |
| `bridge_common/external_agent.rs` | external（codex 等）agent 向け turn state machine・message handling・process 登録/close。`codex.rs` の import 先。 | 補助（A-split） |
| `bridge_common/model_selection.rs` | model 設定・available models 同期・selected model 解決・payload 構築。 | 補助（A-split） |
| `bridge_common/skills.rs` | agent skills scan・slash command / skill entry・image attachment 準備。 | 補助（A-split） |
| `bridge_common/shared.rs` | 上記いずれにも属さない共有定数・helper（`write_bridge_command`・backend config・bridge script 解決・`notify_status_transition`・`GENERATION_COUNTER` 等の crate 内共有要素）。 | 補助（A-split） |

> サブモジュール名・粒度は最終的に実装時に微調整しうる（**仮定 A-naming**）。境界の定義（どの責務がどこに行くか）は上表で固定する。

### 編集（参照のみ・signature 不変）

- `runtime/mod.rs` — 原則無変更。`pub mod bridge_common;` と `pub(crate) use bridge_common::*;` はそのまま機能する（ディレクトリ module でも同一構文）。529 行目の `bridge_common::AgentProcessMap` 参照も維持される。
- `codex.rs` / `claude.rs` — 無変更。`bridge_common::{...}` の import パスが維持されるため。

### 削除

- `bridge_common.rs`（単一ファイル）を削除し、`bridge_common/` ディレクトリへ置換する。

## アーキテクチャと責務分割

### モジュール構成と公開境界

```
runtime/
├── mod.rs                       (pub mod bridge_common; pub(crate) use bridge_common::*;  ← 無変更)
└── bridge_common/
    ├── mod.rs                   (サブモジュール宣言 + 明示 re-export facade)
    ├── process_registry.rs      (runtime / process registry)
    ├── stream_emit.rs           (stream emit)
    ├── session_persistence.rs   (session persistence)
    ├── permission.rs            (permission)
    ├── recovery.rs              (recovery)
    ├── session_lifecycle.rs     (補助)
    ├── turn_event_log.rs        (補助)
    ├── sdk_message.rs           (補助)
    ├── external_agent.rs        (補助)
    ├── model_selection.rs       (補助)
    ├── skills.rs                (補助)
    └── shared.rs                (補助)
```

`bridge_common/mod.rs` の骨子:

```rust
mod process_registry;
mod stream_emit;
mod session_persistence;
mod permission;
mod recovery;
mod session_lifecycle;
mod turn_event_log;
mod sdk_message;
mod external_agent;
mod model_selection;
mod skills;
mod shared;

pub(crate) use process_registry::{AgentProcessMap, TurnPhase};
pub(crate) use session_lifecycle::{get_session, send_agent_message_internal};
pub(crate) use permission::{respond_agent_permission, set_agent_permission_mode};
// ... 外部互換に必要な symbol だけを明示 re-export
```

- 各サブモジュール内の可視性は、外部互換に必要な Tauri command / runtime entrypoint だけを `mod.rs` から明示 re-export する。サブモジュール間で相互参照される helper は、可能な限り `pub(super)` または private / `#[cfg(test)]` に留める（**仮定 A-vis**）。可視性の昇格は必要な呼び出し関係に限定し、facade の API surface は最小に保つ。
- `bridge_common::Foo` という外部参照パスは、既存参照元が実際に使う symbol についてのみ `mod.rs` の明示 re-export で維持する。責務 module 内部の helper は wildcard facade で再公開しない。

### 責務間の依存方向

分割後の依存は「共有基盤 → 個別責務 → lifecycle/message 統合」の概ね一方向に整理する。

- `process_registry`（型）・`shared`（定数/helper）が最下層。他サブモジュールはここに依存する。
- `stream_emit` は coalescing / flush / emit / timer に限定し、`process_registry` の `AgentProcess` を受け取って streaming buffer の配信状態だけを操作する。
- `session_persistence` / `permission` / `recovery` / `turn_event_log` は各責務の状態遷移・永続化・復旧・投影を所有する中間層。
- `sdk_message` / `session_lifecycle` / `external_agent` は上記中間層を呼び出す統合層。SDK message 分類は `sdk_message`、turn complete orchestration は `session_lifecycle`、timeout/error 復旧は `recovery`、permission 応答遷移は `permission` が所有する。

巨大関数（例: `handle_sdk_message`・`complete_streaming_turn_post_lock`・`start_agent_turn_internal_locked`）は複数責務に跨るが、関数本体は「主たる責務（message 受信統合 = `sdk_message`、turn 完了統合 = `session_lifecycle`）」のサブモジュールに置き、他責務の処理は対応サブモジュールの関数呼び出しに委ねる（既存の関数分解をそのまま module 境界に写像する）。新たな関数分割は最小限に留める。

### read / write 境界の明確化（R4）

- read 経路（副作用なしの取得系。`get_session` など）と write 経路（永続化・状態変更を伴う系）を `session_lifecycle.rs` 内で別関数として隣接配置し、コメントまたはモジュール内の節区切りで識別可能にする。
- requirements の非スコープに従い、read command が現在持つ write side-effect の**実除去は行わない**（A4）。本 Issue では「read 経路と write 経路が構造上識別できる」状態の達成に留める。`get_session` の出力は分割前後で同一（behavior.md「read command の観測可能な出力が分割前後で一致する」）。

## データモデルまたは型

新規の型・データ構造は導入しない。分割前に存在する型をそのまま移設する。

- `AgentProcess`（90-169 行）/ `BridgeState`（56-61 行）/ `TurnPhase`（66-73 行）/ `PendingMessage`（76-88 行）/ `AgentProcessMap = HashMap<String, AgentProcess>`（217 行）→ `process_registry.rs`。
- `PersistedSpawnInfo`（pub(crate)）→ `session_lifecycle.rs` または `shared.rs`。
- `ExternalAgentTurnStart` / `ExternalBridgeMessageState` / `ExternalPendingTurn`（pub(crate)）→ `external_agent.rs`。
- レスポンス型 `CancelQueuedTurnResponse` / `SendMessageResponse` / `InitSessionsResponse` / `SlashCommandEntry` / `SkillEntry` → それぞれの責務サブモジュール（`session_lifecycle` / `skills`）。
- 共有 global state `GENERATION_COUNTER: AtomicU64` → `shared.rs`（`pub(crate)`）。複数サブモジュール（spawn・stale event reject）が参照するため。
- 定数群: streaming/emit 系（`STREAMING_EMIT_INTERVAL_MS`・`STREAMING_PENDING_PART_LIMIT`・`STREAMING_PENDING_BYTE_LIMIT`）→ `stream_emit.rs`。persist interval（`PERSIST_INTERVAL_MS`）→ `session_persistence.rs`。recovery/watchdog 系（`STALE_TIMEOUT_SECS`・`STALE_RECOVERY_GRACE_SECS`・`WATCHDOG_TICK_SECS`・`STALE_EXIT_CODE`）→ `recovery.rs`。backend 識別子（`CLAUDE_BACKEND_ID`・`CODEX_BACKEND_ID`・`DEFER_AGENT_SESSION_ID_PERSIST_ON_READY`）→ `shared.rs`（外部 import 先のため facade 再公開で維持）。

型の定義・フィールド・derive・signature は一切変更しない（observable behavior 不変・wire 互換維持）。

## 処理フロー

分割は「機械的移設」を原則とし、実行時フロー（state 遷移パイプライン）は変えない。代表フローと移設先の対応:

- **turn 開始 → streaming → 完了**: `session_lifecycle`（turn 開始/完了統合）→ `recovery`（spawn 要否判定・spawn）→ `stream_emit`（flush/emit）→ `session_persistence`（post-turn base load・persist）→ `turn_event_log`（durable 記録）。各 module の関数呼び出し順序は分割前と同一。
- **permission 要求 → 応答**: `permission`（state 遷移・応答・resolution 記録）が `process_registry` の `AgentProcess` を更新し、`stream_emit` 経由で event emit。
- **session_ready → resume / context 復元**: `session_persistence`（session_id・context carry load）→ `recovery`（mismatch 時 requeue）→ `session_lifecycle`（resume start）。`context_restore.rs` / `runtime_coordinator.rs` への参照は従来どおり。
- **process death → 再 spawn / orphan cleanup**: `recovery`（PID 管理・cleanup・stale 判定・spawn）。#1192 で確立した挙動を維持（behavior.md 該当 Scenario）。

これらの呼び出しはサブモジュール境界を跨ぐが、内部 helper は `pub(super)` を基本とし、`mod.rs` facade からは外部互換に必要な symbol だけを明示 re-export する。

## エラー処理

- 既存のエラー型・`Result` signature・エラー文言・エラー時の emit/persist 挙動を変更しない（R2・behavior.md「エラー挙動が分割前後で一致」）。
- 各サブモジュールが使うエラー型は分割前と同一のものを `use` する。新規エラー型は導入しない。
- `?` 伝播・`match` 分岐・ログ出力（`tracing` 等）は移設のみで、分岐条件・出力内容を変えない。

## テスト方針

### 既存テストの移設（R3 / R5・behavior.md「既存テストが期待値変更なしで pass」）

- 8,978-21,225 行にある `#[cfg(test)] mod tests` 内のテスト群（約 36 テスト関数 + helper）を、検証対象 production 要素が移った先のサブモジュール内 `#[cfg(test)] mod tests` へ移す。
  - persistence 系テスト → `session_persistence.rs`
  - recovery / resume / spawn 系テスト → `recovery.rs`
  - streaming / emit / interval 系テスト → `stream_emit.rs`
  - permission 系テスト → `permission.rs`
  - SDK message accumulation 系テスト（`test_accumulate_*`）→ `sdk_message.rs`
  - external agent 系テスト → `external_agent.rs`
  - model 選択系テスト → `model_selection.rs`
- テスト helper（`pending_message_for_test`・`test_pending_message`・`begin_test_turn_event_log` 等）は、複数サブモジュールから使う場合 `shared.rs` の `#[cfg(test)]` ブロックへ置き、`pub(crate)` で共有する。単一サブモジュールでしか使わないものは当該サブモジュールへ。
- **テストの期待値（assert 値）は一切変更しない**。移設に伴い `use` パスのみ調整する。

### 境界テストの確保（R3・behavior.md「各責務の module に境界テストが存在する」）

- 5 必須サブモジュール（process_registry・stream_emit・session_persistence・permission・recovery）それぞれに、その責務の境界テストが `#[cfg(test)]` で存在する状態にする。既存テストの移設で大半は満たされる。
- 既存テストが移ってこないサブモジュールがあれば、その責務の最小の境界テスト（既存の振る舞いを固定する characterization test）を補完する（A5）。新規テストは「既存挙動の固定」に限り、新たな仕様は導入しない。

### 検証コマンド（R6・behavior.md「Rust 成果物が緑である」）

`src-tauri/` で以下を実行し、いずれも警告・失敗なく通すことを完了条件とする。

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

分割は機械的移設のため、`cargo test` の pass 集合が分割前と一致することを回帰判定の主軸とする。

## 削除候補の整理（R5・#878 接続）

- 分割過程で判明した、frontend から使われない command surface / compat path（旧経路・後方互換専用関数等）を、`docs/specs/issues-1217/` 配下のドキュメント（例: `dead-code-candidates.md`）または #878 が参照できる形の一覧として記録する（**仮定 A-deadlist**: 出力先は実装時に確定。Spec ディレクトリ内 Markdown を既定とする）。
- 本 Issue では**実削除しない**（behavior.md「削除候補は本 Issue では削除されず回帰しない」）。一覧化された経路も本 Issue 完了時点で従来どおり動作する。
- 一覧には各候補について「シンボル名・現在の参照元の有無・削除可否の根拠」を含め、#878 がそのまま実行判断できる粒度にする。

## リスクと代替案

### リスク

- **R-巨大関数の跨り**: `handle_sdk_message` 等の巨大関数が複数責務を内包し、移設先の判断が曖昧になる。→ 「主たる責務へ本体を置き、他責務は関数呼び出しに委ねる」原則で機械的に裁く。関数の内部ロジック自体は分割しない（スコープ膨張防止）。
- **R-可視性昇格による副作用**: サブモジュール分割で private → `pub(crate)` 昇格が必要になり、意図せず crate 内 API surface が広がる。→ `mod.rs` facade は明示 re-export に限定し、内部 helper は `pub(super)` / private / test-only を優先する。surface の実質は外部互換 symbol に限定する。
- **R-テスト移設での取りこぼし**: 12,000 行のテスト移設で `use` 解決漏れ・helper 重複定義が起きる。→ `cargo test` の pass 集合一致を機械的に確認。テスト関数の総数・名前を分割前後で突き合わせる。
- **R-巨大 diff のレビュー困難**: 21,000 行の移動は review しにくい。→ git の rename 検出を効かせるため、可能な限り「行の移動のみ・改変なし」を保ち、内容変更を伴うコミット（可視性昇格・`use` 調整）を分離する（**仮定 A-commit**: コミット分割は実装時方針）。

### 代替案

- **代替 1: 5 ファイル厳守（補助 module なし）**。requirements R1 の 5 責務だけにファイルを限定する案。→ lifecycle/models/skills/external/sdk_message 等の大量コードを 5 責務へ無理に押し込むと、依然として巨大なファイルが残り「変更の局所化」という目的（R1）を達成できない。**採用しない**（補助 module 併設を採る = A-split）。
- **代替 2: ディレクトリ化せず runtime/ 直下に並列ファイルを置く**。`runtime/bridge_process_registry.rs` 等。→ 外部参照パスが `bridge_common::Foo` から `bridge_process_registry::Foo` へ変わり、`codex.rs` / `mod.rs` の import 改変が必要になる。回帰面・差分面で不利。**採用しない**（ディレクトリ化 + facade を採る）。
- **代替 3: 外部パスを `runtime::Foo` へ整理（A6 の「整理」側）**。→ 本 Issue のスコープを超える参照改変を誘発し、observable risk を増やす。本 Issue は「維持」を採り、整理は将来 Issue に委ねる。

## 仮定

- **A-split**: requirements R1 の 5 責務を必須サブモジュールとし、5 責務に属さない既存 production コード（lifecycle・models・skills・external agent・sdk message・turn event log・shared）は補助サブモジュールとして併設する。requirements A1 が design へ委ねた「1 責務 1 ファイルか細分するか」をこの粒度で確定する。
- **A-naming**: サブモジュールのファイル名は本書の表を既定とし、実装時に Rust 慣用へ微調整しうる（責務境界自体は変えない）。
- **A-vis**: サブモジュール間参照のため private 要素を最小限 `pub(super)` / `pub(crate)` へ昇格する。外部公開境界は `bridge_common/mod.rs` の明示 re-export に限定し、内部 helper は facade から再公開しない。
- **A6（requirements 由来）**: `runtime/mod.rs` の `pub mod bridge_common;` / `pub(crate) use bridge_common::*;` 境界は維持するが、`bridge_common/mod.rs` 側は外部互換に必要な symbol のみを明示 re-export する。public command の signature・名前・呼び出し経路は不変。
- **A-deadlist**: 削除候補一覧の出力先は Spec ディレクトリ内 Markdown（`dead-code-candidates.md` 等）を既定とする。
- **A-commit**: 巨大 diff の review 性のため、「純粋な行移動」と「可視性昇格・`use` 調整」をできる限り別コミットに分離する。

## Open Questions

なし。
