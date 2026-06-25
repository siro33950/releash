# Design

対象 Issue: #1214 — Agent ストリーミング配信を「累積 snapshot」から「`seq` 付き delta（通常配信）＋ resync 時 snapshot」へ移行する。

本書は `requirements.md` / `behavior.md` を受け、実コードを確認したうえで実装方針・責務分割・データ構造・処理フロー・エラー処理・テスト方針を確定する。requirements / behavior が `design.md` に委ねた項目（A2/A3/A6 等）を本書で確定する。

## 概要

現状の Agent streaming は、約 33ms ごとに `streaming_parts`（当該ターンの累積 parts）全体を `consolidate_parts_from_slice` で統合し直し、Tauri event `agent-streaming-updated` と `WsBroadcaster::send_stream_sync`（`AgentStreamSync { session_id, message_id, parts }`）の双方で **その時点の累積 parts 配列全体** を送る。応答が長くなるほど 1 frame あたりの clone / consolidate / serialize / payload が応答全体長に比例して増える。

本設計では通常配信を **`seq` 付き delta**（前回 emit 以降に新たに生じた増分のみ）に置き換える。reconnect / resync 時にのみ **`seq` 付き snapshot**（現時点の累積 parts 全体）を送り、受信側が `since_seq` を起点に最終状態へ復元できるようにする。delta の生成・seq 採番・適用・重複排除・順序づけ・resync 復元の正典ロジックは Rust（`bridge_common` の共有モジュール）に置く。

設計の核心となる判断:

- **delta = 「前回 emit 以降に蓄積した pending payload」**。`pending_stream_parts` が前回成功 emit 以降の delta payload を保持し、pending 件数は `pending_stream_parts.len()` から導出する。累積総量に依存せず増分のみが payload になる（R1）。
- **`seq` は `(session_id, message_id)` 単位の単調増加 u64 カウンタ**。1 delta（1 配信単位）ごとに +1。受信側は seq の連続性で欠落を検知し、適用済み seq 以下の delta を冪等に無視する（R2）。
- **resync snapshot の復元源は新規バッファを持たない**。アクティブターン中は `streaming_parts` がそのまま累積 parts を保持しているのでそこから snapshot を生成し、ターン完了後（#1194 が `streaming_parts` を解放済み）は永続化済み message から生成する。delta 履歴バッファは導入しない（R7）。
- **frontend は delta を受信して append・display 整形するのみ**。順序づけ・重複排除・resync 復元の正典は Rust 側。frontend reducer の delta 適用は「受信 delta parts の追記＋末尾同型 text/thinking の連結（表示整形）＋ seq による冪等ガード／欠落検知トリガ」に限定し、欠落検知時は Rust の resync 経路を呼ぶ（R9。詳細は「仮定 A3」）。

## 変更対象

### Rust（src-tauri）

- `src-tauri/src/protocol/agent.rs`
  - `AgentStreamSync` に `seq: u64` を追加（resync snapshot 専用の意味へ変更）。
  - 新規 `AgentStreamDeltaMsg { session_id, message_id, seq, parts: Vec<AgentStreamPartMsg> }`（通常配信 = delta）。
  - 新規 `ResyncStreamReq { session_id, message_id, since_seq: u64 }`（client→server の resync 要求）。
- `src-tauri/src/protocol/mod.rs`
  - `WsMessage` に variant 追加: `AgentStreamDelta(AgentStreamDeltaMsg)`、`ResyncStream(ResyncStreamReq)`。`AgentStreamSync` は resync snapshot 配信用として残す。
- `src-tauri/src/infrastructure/agent_session/runtime/bridge_common/stream_emit.rs`
  - `emit_streaming_parts` を「delta 用」と「snapshot 用」に分ける（または `EmitKind` を引数化）。通常 flush は delta を emit、resync 要求時のみ snapshot を emit。
  - `prepare_streaming_flush` の出力を「増分スライス（delta）」へ変更し、seq を採番。
  - `StreamingFlushSnapshot` に `seq`・`delta_parts`（増分）を追加。
- `src-tauri/src/infrastructure/agent_session/runtime/bridge_common/shared.rs`（または新規 `stream_delta.rs`）
  - delta 抽出（増分スライスの consolidate）・seq 採番ヘルパ・resync snapshot 生成（accumulate 適用の参照実装）・受信側適用の参照実装（テスト用）を集約。`consolidate_parts_from_slice` は流用。
- `src-tauri/src/infrastructure/agent_session/runtime/.../process_registry.rs`
  - `AgentProcess` に `streaming_delta_seq: u64`（`(session, message)` 単位の seq カウンタ）を追加。`reset_streaming_state_for_new_turn` でリセット。
- `src-tauri/src/ws_bridge.rs`
  - `send_stream_delta`（ordered・lossless キュー）と `send_stream_snapshot`（resync）を追加。slow-consumer / overflow 時の snapshot フォールバック（後述）。`drain_*` を更新。
- `src-tauri/src/ws_server/session.rs` / `routing.rs`
  - `route_message` を stub から脱却させ、`ResyncStream` を受けて該当 message の resync snapshot を push する経路を実装。forward task が delta / snapshot の両キューを drain。
- Tauri command（`src-tauri/src/.../agent_session` の command 層）
  - frontend からの resync 用 command（例 `resync_streaming_message(session_id, message_id, since_seq)`）を追加。Rust read model が現時点の authoritative snapshot+seq を返す。

### Frontend（src）

- `src/types/protocol.ts`
  - `AgentStreamSync` に `seq` 追加。`AgentStreamDelta`・`ResyncStreamReq` 型追加。`WsMessage` union 更新。
- `src/hooks/useAgentSdkListeners.ts`
  - 新 Tauri event `agent-streaming-delta`（delta）を listen。delta は新 action へ dispatch し、seq 欠落／message 未キャッシュ時に resync command を呼ぶ。resync snapshot は `resync_streaming_message` の command response を既存 `SET_STREAMING_MESSAGE`（丸ごと置換）へ dispatch する。未実装の独立 Tauri event `agent-streaming-snapshot` は設けない。
- `src/hooks/agentChatReducer.ts`
  - 新 action `APPLY_STREAMING_DELTA { sessionId, messageId, seq, parts }`。受信 delta parts を当該 message の parts 末尾へ追記し、末尾同型 text/thinking を連結（表示整形）。`lastStreamingSeqByMessage` を保持して冪等ガード／欠落検知。`SET_STREAMING_MESSAGE`（snapshot）適用時に seq を更新。

## アーキテクチャと責務分割

### 配信の意味論（確定）

| 配信種別 | トリガ | 運ぶもの | frontend 経路 | WsMessage |
|---|---|---|---|---|
| 通常配信 | 33ms coalescing flush / 閾値 early flush / tool 境界 / 状態遷移前 flush | `seq` 付き **増分 delta** | `agent-streaming-delta` | `AgentStreamDelta` |
| resync | reconnect / seq 欠落検知 / message 未キャッシュ | `seq` 付き **累積 snapshot** | `resync_streaming_message` command response | `AgentStreamSync` |

通常配信中は snapshot を送らない（behavior「通常配信中は snapshot を送らない」）。

### 責務（Rust が正典）

- **delta 生成・seq 採番**: `stream_emit` + 共有モジュール。`prepare_streaming_flush` が `pending_stream_parts` を consolidate して delta を作り、`streaming_delta_seq += 1` で seq を採番。
- **resync snapshot 生成**: 共有モジュール / read model。アクティブターン中は `streaming_parts` を consolidate、完了後は永続化済み message から再構成。`since_seq` より新しい seq の snapshot を返す。
- **適用・順序づけ・重複排除の参照実装**: 共有モジュールに「delta を seq 順に accumulate して最終 parts を得る」純関数を置き、ユニットテストの基準にする。WS 受信クライアントは不在のため、この参照実装が WS 側適用の正典かつ被テスト対象（behavior A5）。
- **表示整形のみ frontend**: delta parts の追記、末尾同型 text/thinking の連結（display）、seq 整数比較による冪等ガード／欠落検知トリガ。実体の復元は Rust の resync 経路に委譲。

### R9 解釈（重要・仮定として明示）

R1 は「1 配信単位（= frontend への Tauri event を含む）の payload が応答全体長に比例しない」ことを要求するため、frontend へは累積ではなく delta を届ける必要がある。よって frontend は delta を「適用」せざるを得ない。本設計は R9 を次のように解釈する:

- **正典（authoritative）な delta 生成・順序づけ・重複排除・resync 復元のロジックは Rust** に置き、ユニット／結合テストで検証する。
- **frontend の delta 適用は最小・機械的な表示整形**（追記＋同型連結＋ seq の整数比較）に限定し、ビジネスロジック・復元ロジックを持ち込まない。欠落・未キャッシュ時の状態復元は必ず Rust（resync command / read model）に委ねる。

この境界を `仮定 A3` として記録する。

## データモデルまたは型

### protocol（Rust）

```rust
// protocol/agent.rs

// 通常配信 = 増分 delta
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStreamDeltaMsg {
    pub session_id: String,
    pub message_id: String,
    pub seq: u64,                       // (session_id, message_id) 単位で単調増加
    pub parts: Vec<AgentStreamPartMsg>, // このフレームの増分のみ（累積ではない）
}

// resync snapshot = 累積（seq を付与）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStreamSync {
    pub session_id: String,
    pub message_id: String,
    pub seq: u64,                       // この snapshot が表す到達 seq（追加フィールド）
    pub parts: Vec<AgentStreamPartMsg>, // 累積 consolidated parts
}

// client → server の resync 要求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResyncStreamReq {
    pub session_id: String,
    pub message_id: String,
    pub since_seq: u64,                 // この seq までは適用済み
}
```

`seq` は `serde(default)` を付与し、未送出側との後方互換（欠損時 0）を確保する。protocol versioning は現状なし・additive 方針を踏襲（既存 variant は不変、新 variant 追加のみ）。

### AgentProcess（Rust）

```rust
// 既存（流用）
pub streaming_parts: Vec<MessagePart>,        // 当該ターンの累積（resync 復元源・アクティブ時）
pub streaming_message_id: Option<String>,
pub(crate) pending_stream_parts: Vec<MessagePart>, // 前回成功 emit 以降の delta payload（件数は len() から導出）
pub(crate) pending_stream_bytes: usize,
pub(crate) last_stream_emit_at: Option<Instant>,
pub last_message_id: Option<String>,          // post-turn 継続用

// 新規
pub(crate) streaming_delta_seq: u64,          // (session, message) 単位の seq。新ターンで 0 にリセット
```

### frontend（TS）

```typescript
// types/protocol.ts
export interface AgentStreamSync { session_id: string; message_id: string; seq: number; parts: MessagePart[] }
export interface AgentStreamDelta { session_id: string; message_id: string; seq: number; parts: MessagePart[] }
export interface ResyncStreamReq { session_id: string; message_id: string; since_seq: number }

// agentChatReducer.ts: 新 action
| { type: "APPLY_STREAMING_DELTA"; sessionId: string; messageId: string; seq: number; parts: MessagePart[] }
// state に追加: lastStreamingSeqByMessage: Record<string /*messageId*/, number>
```

## 処理フロー

### 通常配信（delta）

1. SDK chunk 到着 → `append_to_parts` が `streaming_parts` に fine-grained part を push し、同じ delta payload を `pending_stream_parts` に append。
2. flush 判定（既存の `should_flush_per_delta`：post-turn / tool 境界 / 閾値 / interval）。
3. `prepare_streaming_flush`：`pending_stream_parts.is_empty()` なら None。そうでなければ `pending_stream_parts` を `consolidate_parts_from_slice` で統合し、`streaming_delta_seq += 1` で seq を採番、`AgentStreamDeltaMsg` を組む。
4. `emit_streaming_delta`：
   - Tauri event `agent-streaming-delta` を emit。
   - `WsBroadcaster::send_stream_delta`（ordered キュー）へ。
5. `apply_streaming_emit_result`：成功なら pending を 0 にして seq 確定、失敗なら pending を保持（次 flush で同じ増分を再送＝同一 seq の再送ではなく未確定のまま。seq は emit 成功時にのみ確定するよう採番位置を調整。下記「エラー処理」参照）。

> delta の連結境界: 増分スライスの先頭 part が直前に emit 済みの末尾 part と同型（text/thinking、同一 `parent_tool_use_id`）の場合、送信側はその increment を独立 part として送り、**受信側が自分の末尾 part へ連結する**。これにより累積 consolidated 結果（変更前 snapshot）と一致する（R8）。

### 受信・適用（frontend）

1. `agent-streaming-delta` 受信 → 当該 message の `lastStreamingSeqByMessage[messageId]` を確認。
   - `seq <= last`：適用済み → 無視（冪等、R2）。
   - `seq == last + 1`：`APPLY_STREAMING_DELTA` を dispatch（追記＋末尾同型連結）、`last = seq`。
   - `seq > last + 1`（欠落）または message 未キャッシュ：resync command を呼び snapshot 取得。
2. `resync_streaming_message` の command response（resync 結果）受信 → `SET_STREAMING_MESSAGE`（parts 丸ごと置換）＋ `last = snapshot.seq`。以降は通常 delta へ戻る。

### resync（frontend → Rust）

1. frontend が `resync_streaming_message(session_id, message_id, since_seq)` を invoke。
2. Rust read model：
   - 当該 message がアクティブターン中（`streaming_message_id == message_id`）→ `streaming_parts` を consolidate、現 `streaming_delta_seq` を seq に。
   - 完了済み → 永続化済み message（session store）から parts を再構成、最終 seq を付与。
3. `AgentStreamSync`（snapshot）を command response として返す。

### resync（WS）

- client → server `ResyncStream(ResyncStreamReq)` を `route_message` が受け、同じ read model で snapshot を生成し `AgentStreamSync` を push。受信クライアントは不在のため経路と整合のみを Rust 結合テストで検証（A5）。

### WS 配信のメモリ有界化（delta 化への対応）

現状の `latest_stream_sync` slot 上書きは「各 snapshot が cumulative なので中間を落としても可」という前提に依存する。delta は欠落不可のため、`WsBroadcaster` を次のように設計する:

- 通常は **per-message の順序付き delta キュー**（lossless）に push。
- キューが上限（件数 / bytes）を超過、または (re)connect 直後は、**当該 message の queued delta を破棄して resync snapshot 1 件に畳む**（cumulative なのでメモリ有界に戻る）。受信側は snapshot で `seq` ごと最終状態へ収束。
- これにより「通常は小さな delta 列、バックプレッシャ時は snapshot 1 件」でメモリは `(live streaming messages × 1 snapshot)` に有界化（既存の bounded 特性を維持）。

## エラー処理

- **emit 失敗（transport）**: seq は emit 成功時に確定する。`prepare_streaming_flush` で seq を仮採番せず、`apply_streaming_emit_result` の成功時に `streaming_delta_seq` を確定インクリメントする（失敗時は同一増分を次フレームで再試行し、seq の穴を作らない）。Tauri と WS のどちらか一方失敗時は既存同様 pending を保持して再送。
- **seq 欠落**: 受信側は欠落検知で resync を要求し、snapshot で収束（破綻しない、R4）。
- **重複 seq**: 受信側は `seq <= last` を無視（冪等、R2）。
- **out-of-order**: in-process Tauri / 単一 TCP の WS では基本的に順序保証。欠落として観測された場合は resync で一意収束（AC2）。
- **message 未キャッシュ**（既存 `hasMessage` ミス相当）: delta 適用先が無い → resync command で session/snapshot を hydrate。既存の `getSession` フォールバックは維持しつつ、focused resync を優先。
- **post-turn 継続**: ターン完了後の background イベントは `last_message_id` の message に対して delta を継続発行する。`streaming_delta_seq` はターン完了で **リセットしない**（リセットは `reset_streaming_state_for_new_turn`＝新ターン開始時のみ）。`streaming_parts` は #1194 が完了時に解放するため、post-turn delta は再び空からの append となる。post-turn delta の resync は「アクティブでない」ため永続 message ＋ post-turn 増分の合成で復元する。この経路はエッジテストで担保する。

## テスト方針

Rust（共有モジュール／protocol／ws_bridge）を中心に、純関数化したロジックへ単体テストを集中させる（R9・behavior A5）。

- **delta 抽出**: 増分スライスが累積総量でなく増分のみを含むこと、consolidate が境界をまたいで誤連結しないこと（R1/AC1）。
- **適用・冪等・順序**: delta 列を seq 順 accumulate した最終 parts が、変更前の累積 snapshot consolidate と一致（R8/AC2）。同一 seq 重複適用が冪等。欠落 seq を検知できる。順序入れ替え → resync で一意収束。
- **resync 復元**: `since_seq=N` から snapshot を適用した最終状態が変更前と一致（R3/R8/AC3）。アクティブ時（`streaming_parts` 源）・完了後（永続 message 源）の両系。
- **#1194 整合**: delta 化が `streaming_parts` の累積常駐／delta 履歴バッファを再導入しないこと（R7）。完了後に `streaming_parts` 解放済みでも resync が成立すること。
- **parts_to_legacy 限定**: delta / snapshot の payload と保存正典が `parts` ベースで、`parts_to_legacy` を経由しないこと（R6/AC5）。`parts_to_legacy` は互換出力でのみ呼ばれることをコード／テストで確認。
- **WS protocol / routing**: `AgentStreamDelta` / `AgentStreamSync` / `ResyncStream` の serialize/deserialize、`route_message` の resync 応答、backpressure 時の snapshot フォールバック（A5）。
- **frontend reducer**: `APPLY_STREAMING_DELTA` の追記・末尾同型連結、seq 冪等ガード、欠落／未キャッシュ時の resync トリガ、snapshot 適用後の delta 復帰。`useAgentSdkListeners` の event 振り分け。
- **表示即時性（R5/AC4）**: 33ms coalescing と tool 境界 flush の即時 emit が維持されること（既存 timer/flush テストの非退行）。
- **green 維持（R10/AC6）**: `cargo test` / `pnpm test` / `cargo clippy -D warnings` / `pnpm lint`。

## リスクと代替案

- **R9 と R1 の緊張**: frontend が delta を適用せざるを得ない。本設計は「正典は Rust、frontend は最小機械的整形」で解消（仮定 A3）。代替案として「Rust read model が状態を保持し frontend には常に snapshot を再 emit」する案があるが、これは R1（frontend 配信 payload が累積比例しない）を満たせないため不採用。
- **WS delta のメモリ有界性**: cumulative slot 上書きの利点が delta で失われる。backpressure 時 snapshot 畳み込みで再確保（上記）。代替として無制限 ordered キューは、停滞時にターン長相当まで成長しうるため不採用。
- **seq の確定タイミング**: emit 失敗時に seq の穴を作らないため「成功時確定」を採用。prepare 時仮採番＋失敗ロールバックは状態が複雑になるため不採用。
- **post-turn 継続の複雑性**: seq を完了時にリセットしない方針で連続性を保つが、`streaming_parts` 解放との合成が複雑。エッジテストで担保し、破綻時は resync で収束させる安全弁を持つ。
- **protocol 互換**: WS 受信クライアントは削除済みのため破壊的変更の即時リスクは低い。`seq` は `serde(default)` で additive に保つ。

## 仮定

- A1: 本 Issue は #1194（turn 完了時の `streaming_parts` 解放）の成果を前提とする。解放実装そのものは #1194 が owner。
- A2: `seq` は `(session_id, message_id)` 単位の単調増加 u64 で、1 配信単位（1 delta）ごとに +1。採番起点は新ターン開始（message 確定）時に 0、最初の delta が seq=1。`since_seq` は「その seq まで適用済み」を意味する。
- A3（R9 解釈）: 正典な delta 生成・順序づけ・重複排除・resync 復元ロジックは Rust（`bridge_common` 共有 / read model）に置く。frontend の delta 適用は「追記＋末尾同型連結（表示整形）＋ seq 整数比較による冪等ガード／欠落検知トリガ」に限定し、状態復元は Rust の resync 経路へ委譲する。R1 が frontend への delta 配信を要求するため、frontend での delta 適用そのものは不可避であり、その範囲を表示整形に閉じることで R9 を満たすものとする。
- A4: resync snapshot の復元源は、アクティブターン中は `streaming_parts`、完了後は永続化済み message とする。delta 履歴バッファや完了ターン分の常駐は導入しない（R7 整合）。#1213 の paging を完了後復元に利用するのは可だが本 Issue では必須としない。
- A5: 通常配信は新 Tauri event `agent-streaming-delta` ＋ WsMessage `AgentStreamDelta`、frontend resync は `resync_streaming_message` command response、WS resync は WsMessage `AgentStreamSync`（`seq` 付与）とする。既存 `agent-streaming-updated` event は本変更で `agent-streaming-delta` と resync command response へ置き換える（累積 snapshot の常時配信は廃止）。
- A6: WS 受信クライアントは削除済みのため、WS 経路は protocol / サーバー側（broadcaster・routing・resync 応答）の整備までを範囲とし、受信クライアントを介した E2E は行わず Rust 単体・結合テストで検証する。
- A7: 「外部から観測可能な振る舞い」は受信 message store に最終適用される parts・表示即時性・resync 後収束状態を指し、payload 形状・送信回数・clone 回数・メモリ常駐量は含めない。

## Open Questions

なし。
