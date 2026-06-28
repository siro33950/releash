# Design

## 概要

本 ISSUE は milestone [12]（クリーンアーキテクチャ移行）の一環として、agent session の
streaming delta 順序付け・session state・workflow step status 集約の source of truth を
frontend reducer / listener / utility から backend-owned read model へ確定させる。

requirements.md の事実調査どおり、対象は次の 4 系統に分かれる。

- **A. raw stream-json parser / ANSI 整形（dead code 削除）**
- **B. worktree 単位 agent state 集約 utility（dead code 削除）**
- **C. streaming delta の ordering / 重複 / drop / resync 所有を backend へ集約（配信契約ごと backend 主導へ）**
- **D. workflow step status の version dedup / 集約を Rust query 側へ移管**

A / B は production 未消費 dead code の削除であり、観測可能な振る舞いを変えない。
C / D は所有境界の移動であり、外部から観測可能な振る舞い（streaming 表示結果・turn phase・
permission pending・queue・step status 表示）は不変に保つ。

backend は既に streaming delta の `seq` 付与、`resync_streaming_message` read model、
`streaming_final_seq` 永続化、`agent-streaming-delta` / `agent-session-state-changed` /
`workflow-step-status-changed` の発火、`AgentStatusCenter` による workflow step version 管理を
所有している。本設計は「backend が確定済みの順序・集約結果を push し、frontend はそれを mirror として
反映するだけ」という配信契約へ寄せ、frontend に残る二重判断を撤去する。

## 変更対象

### frontend（削除）

| ファイル | 操作 | 根拠 |
|---|---|---|
| `src/lib/parseStreamJson.ts` | 削除 | production 消費者なし（参照は test のみ）。スコープ A |
| `src/lib/parseStreamJson.test.ts` | 削除 | 対象実装と同時削除 |
| `src/lib/stripAnsi.ts` | 削除 | production 消費者は dead な `parseStreamJson` のみ。スコープ A |
| `src/lib/stripAnsi.test.ts` | 削除 | 対象実装と同時削除 |
| `src/lib/agentStateUtils.ts` | 削除 | production 消費者なし（参照は test のみ）。スコープ B |
| `src/lib/agentStateUtils.test.ts` | 削除 | 対象実装と同時削除 |

### frontend（縮小）

| ファイル | 操作 | 内容 |
|---|---|---|
| `src/hooks/agentChatReducer.ts` | 縮小 | `APPLY_STREAMING_DELTA` の seq 比較 / drop / `lastStreamingSeqByMessage` 保持を撤去。snapshot 反映を `SET_STREAMING_MESSAGE` へ一本化。スコープ C |
| `src/hooks/useAgentSdkListeners.ts` | 縮小 | gap 検出・`resync_streaming_message` 駆動・`getEffectiveLastSeq` / `advanceOptimisticLastSeq` / `hydrateMessageIfMissing` / `dispatchResyncSnapshot` を撤去し、最小 listen 配線へ。スコープ C |
| `src/hooks/useWorktreeStepStatuses.ts` | 縮小 | `stepVersions` / `workflowVersions` の version dedup と Map 集約を撤去し、backend 集約済み read model を保持・描画。スコープ D |
| `src/hooks/useAgentChat.ts` | 微修正 | 削除した ref（`lastStreamingSeqByMessageRef` 等）・撤去した listener 引数の配線整理。スコープ C |

### backend（変更）

| ファイル | 操作 | 内容 |
|---|---|---|
| `src-tauri/src/infrastructure/agent_session/runtime/bridge_common/stream_emit.rs` | 変更 | in-place update / overflow 時に「frontend が resync を駆動する seq gap」を作らず、backend が snapshot event を自発 emit する配信へ変更。スコープ C |
| `src-tauri/src/adaptor/protocol/agent.rs` | 変更 | delta payload に「append か snapshot か」を判別するフィールドを追加（後述）。スコープ C |
| `src-tauri/src/usecase/agent_session/status.rs` | 変更 | worktree 単位の集約済み step status read model を返す query を公開。スコープ D |
| `src-tauri/src/adaptor/controller/command/agent_session/` | 変更 | step status 集約 query の Tauri command を追加。frontend resync command 経路の扱いを整理。スコープ C/D |
| `src-tauri/src/adaptor/presenter/agent_status.rs` | 変更 | `workflow-step-status-changed` の payload を集約済み representative snapshot 形へ寄せる。スコープ D |

> `resync_streaming_message` read model（`stream_resync.rs` / `runtime_gateway.rs`）は backend 内 replay 実装として温存する（WS reconnect / snapshot emit に再利用）。撤去するのは **frontend からの駆動経路** であって backend の replay 能力ではない。

## アーキテクチャと責務分割

### 所有境界（移行後）

| 概念 | 移行後の所有者 | frontend の責務 |
|---|---|---|
| streaming delta の順序・重複・drop | backend（runtime stream_emit） | 到着順にそのまま反映 |
| streaming 欠損補完 / replay | backend（resync read model を内部利用） | 反映のみ。自発 resync しない |
| turn phase / permission pending / queue | backend（session state、既存） | `agent-session-state-changed` を反映（既存維持） |
| workflow step status の version dedup / representative 選択 | backend（`AgentStatusCenter`） | 集約済み read model を反映 |
| active session id / panel 開閉・選択 / invoke・listen 配線 | frontend（維持） | 既存どおり |

### C の配信契約（中心設計）

現状は「append-only は seq+1、in-place update は seq+2（gap）」で **gap を frontend への resync 合図**として使い、
frontend が `resync_streaming_message` を呼んで snapshot を取り直す двойного判断になっている。
これを「**backend が常に到着順で自己充足な event を push する**」契約へ変える。

emit する event を 2 種に明確化する（チャネルは既存 `agent-streaming-delta` を踏襲）。

1. **append event** — 既存 streaming message の末尾へ parts を追記する差分。
2. **snapshot event** — その message の parts 全体を置換する確定スナップショット。
   in-place update（従来 seq+2 gap を作っていたケース）・overflow collapse・tool output 永続化後の
   resync（`emit_persisted_tool_output_resync`）は、すべて snapshot event として emit する。

frontend は event 種別だけを見て、append なら追記、snapshot なら置換する。
**seq の連続性・重複・gap を一切判定しない。** Tauri event は単一プロセス内・単一チャネルで順序保証されるため、
到着順 == backend 確定順となる。WS 経路も broadcaster が順序保持し、overflow 時は既存の
collapse-to-snapshot で snapshot event に畳む。

`seq` フィールド自体は payload に残してよいが（観測・ログ・将来の WS 整合用）、
**frontend の適用判断には使わない**。frontend reducer から `lastStreamingSeqByMessage` を削除する。

reconnect / replay（主に将来の WS client）は backend が `resync_streaming_message` read model を
内部利用して snapshot event を送ることで完結させる。desktop（Tauri）には disconnect 概念がないため、
session load 時の既存 `getSession` snapshot ＋ 上記 snapshot event で整合する。

### D の集約契約

`AgentStatusCenter`（`usecase/agent_session/status.rs`）が既に
`workflow_steps: RwLock<HashMap<WorkflowStepKey, WorkflowStatusEntry{representative, version}>>` と
atomic version を所有しており、dedup / representative 解決の正典は backend にある。

frontend の version dedup を不要にするため、backend は次を提供する。

- **集約 sync command**: worktree 単位で「解決済み representative の集合」を
  `workflow-step-status-changed` event として emit する void command。
  初回購読・worktree 切替時に frontend が listen 登録後に invoke し、戻り値は読まない。
- **変更 event**: `workflow-step-status-changed` の payload を、`AgentStatusCenter` のロック下で
  解決した **集約後 representative snapshot**（該当 worktree の step / workflow representative の現在値）に
  寄せる。frontend は payload をそのまま該当 worktree の map へ反映するだけにする。

これにより frontend は version 比較・representative 選択を行わない。初回 snapshot と live 更新は
同じ ordered event チャネルで供給され、古い version の巻き戻り防止は backend の dedup / representative
解決が担う。

## データモデルまたは型

### C: streaming event payload（`adaptor/protocol/agent.rs`）

`AgentStreamDeltaMsg` に event 種別判別を追加する。実装簡潔さのため真偽フラグを採用する。

```rust
pub struct AgentStreamDeltaMsg {
    pub session_id: String,
    pub message_id: String,
    pub seq: u64,        // 観測・WS 整合用に保持。frontend の適用判断には使わない
    pub snapshot: bool,  // true: parts 全置換(snapshot) / false: 末尾追記(append)
    pub parts: Vec<AgentStreamPartMsg>,
}
```

- 既存 `AgentStreamSync`（snapshot 専用 DTO）と二重定義にせず、`agent-streaming-delta` チャネルでは
  `AgentStreamDeltaMsg { snapshot: true, .. }` を snapshot として表現する。
- frontend 型（`src/types`）にも `snapshot: boolean` を追加し、reducer dispatch を分岐する。

### D: worktree step status read model

```rust
// usecase/agent_session/status.rs
pub struct WorktreeStepStatusView {
    pub steps: Vec<WorkflowStepRepresentative>,     // step key + 解決済み status
    pub workflows: Vec<WorkflowRepresentative>,     // execution id + 解決済み status
}
```

- `AgentStatusCenter::query_worktree_step_statuses(worktree_path) -> WorktreeStepStatusView` を公開。
- `sync_worktree_step_statuses(worktree_path) -> ()` は live 更新と同じ ordered emission
  lane 上で同 view を取得し、`workflow-step-status-changed` event として emit する。
- frontend `WorktreeStepStatuses`（`Map<string, WorkspaceStepStatus>` 2 本）は、この view を
  そのまま Map 化して保持する（version Map は持たない）。
- `workflow-step-status-changed` event の payload は初回 sync / live 更新とも同 view で送る。

## 処理フロー

### C: streaming（移行後）

1. runtime stdout reader が delta を蓄積し flush 判定（既存）。
2. `prepare_streaming_flush` が parts を確定。append-only なら append event、
   in-place update を含むなら snapshot event を構築する（従来 seq+2 gap の代わり）。
3. `stream_emit` が `agent-streaming-delta`（`snapshot` フラグ付き）を Tauri emit ＋ WS broadcast。
   overflow collapse・tool output resync も snapshot event として emit。
4. frontend `useAgentSdkListeners` は event を受信し、`snapshot` フラグで分岐:
   - `snapshot: true` → `SET_STREAMING_MESSAGE`（parts 全置換）
   - `snapshot: false` → `APPLY_STREAMING_DELTA`（末尾追記）
   - **seq 比較・gap 検出・resync 呼び出しは行わない。**
5. reducer は受け取った内容をそのまま反映。`lastStreamingSeqByMessage` は廃止。
6. message 完了時の `streaming_final_seq` 永続化（既存）はそのまま。

### D: workflow step status（移行後）

1. worktree 表示開始 / 切替時、frontend が `workflow-step-status-changed` を listen してから
   `sync_worktree_step_statuses` を invoke し、backend が live 更新と同じ ordered emission
   lane で全量 snapshot event を emit。
2. `AgentStatusCenter` の status 変化時、backend がロック下で representative を解決し、
   `workflow-step-status-changed` に集約後 snapshot を載せて emit。
3. frontend は初回 / live の区別なく payload をそのまま該当 worktree の `steps` / `workflows`
   Map へ到着順に反映（version 比較なし）。

## エラー処理

- **A/B（削除）**: 削除後に残存 import がないことを `pnpm lint` / `pnpm build` / `tsc` で機械的に検出する。
  万一 production 参照が見つかった場合は dead code 前提が崩れるため、削除せず実装を再調査する
  （requirements の「production 未消費」前提のガード）。
- **C（配信）**: snapshot event 構築失敗・emit 失敗は既存の retry 経路（`retry_stream_delta`）を踏襲する。
  frontend は失敗時の resync を駆動しないため、欠損が起きても backend の次回 flush / snapshot で収束する。
  module ごとの専用 error type（Rust 規約）を維持する。
- **D（query）**: backend query は `WorktreeStepStatusView` を直接返す。frontend の再同期要求も
  backend が現行 view を `WorktreeStepStatusSync` に変換して返す契約とする。frontend 側で別の
  代替 read model を組み立てる責務は持たない。

## テスト方針

### Rust（責務移管先）

- **C**: `stream_emit` / runtime bridge に対し、append event の連続適用・in-place update が snapshot event に
  なること・overflow collapse が snapshot になること・reconnect replay（`resync_streaming_message` 内部利用）が
  確定 parts を返すことを検証する unit test を追加する。seq ordering / duplicate / drop / replay の
  正典が backend にあることをここで担保する。
- **D**: `AgentStatusCenter` の version dedup（古い version で representative が巻き戻らない）・
  `query_worktree_step_statuses` の集約結果を検証する unit test を追加する。
- いずれも該当 module 内 `#[cfg(test)] mod tests` に配置（既存規約）。
- 品質ゲート: `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`。

### frontend（描画・配線へ縮小）

- 削除する 3 utility の test は実装ごと削除する（責務は Rust test へ移管）。
- `agentChatReducer` test: snapshot 反映（置換）・append 反映（追記）が `snapshot` フラグに従うこと、
  seq 判定が無いことを検証する形に縮小。
- `useAgentSdkListeners` test: `agent-streaming-delta` 受信 → 正しい action dispatch、
  resync invoke を呼ばないことを検証（mock）。
- `useWorktreeStepStatuses` test: listen 登録後の sync command invoke、sync command の戻り値を
  読まないこと、event payload を到着順でそのまま反映すること、version 比較を行わないことを検証。
- 期待値は実装に合わせて緩めず、観測可能な振る舞い（最終 streaming 内容・turn phase・permission・queue・
  step status 表示）の不変を維持する。
- 品質ゲート: `pnpm lint` / `pnpm test` / `pnpm build`。

## リスクと代替案

- **R1: snapshot event 多発による帯域増**。in-place update を毎回 snapshot 化すると payload が増える。
  → 影響は in-place update を含む flush に限定され、従来も gap 後に frontend が full snapshot を resync 取得
  していたため総量は概ね同等以下。問題があれば snapshot に since-base の差分表現を後続最適化として検討（本 ISSUE 外）。
- **R2: Tauri event 順序保証への依存**。frontend が順序を信頼するため、emit 経路が単一チャネル順序を
  保つ前提に依存する。→ Tauri の `app.emit` は単一 in-process チャネルで順序保持。WS は broadcaster が
  順序保持＋overflow collapse で担保。test で append→snapshot の適用順を固定検証する。
- **R3: D の event 全量 snapshot 化のコスト**。worktree 内 step 数が多いと event payload が膨らむ。
  → representative のみの軽量 view であり、step 数は実用上小さい。必要なら差分 event ＋ backend dedup の
  ハイブリッドに切替可能（代替案）。
- **代替案（C, 不採用）**: seq を frontend に残し「単調増加なら適用」だけ残す案。
  → frontend に seq decision が残り requirements C に反するため不採用。判定を完全に撤去する。
- **代替案（D, 不採用）**: event は従来の per-step change のまま、frontend が backend query を毎回再取得。
  → full-recompute 経路を frontend に増やすため不採用。backend が解決済み snapshot を push する形を採る。

## 仮定

- `parseStreamJson` / `stripAnsi` / `agentStateUtils` は production 未消費 dead code であり、
  Rust に同等機能を新設せず削除する（#1303 の方針に倣う）。削除前に grep / build で消費者不在を再確認する。
- streaming event 種別は `AgentStreamDeltaMsg.snapshot: bool` で表現し、既存 `agent-streaming-delta`
  チャネルを踏襲する（新チャネルは追加しない）。`seq` は残すが frontend の適用判断には使わない。
- `resync_streaming_message` read model（`stream_resync.rs` / `runtime_gateway.rs`）は backend 内 replay として
  温存し、frontend からの駆動経路（invoke）と Tauri command wrapper は撤去する。WS / 将来 client は
  usecase read model を直接利用する。
- D の transport は「worktree 単位の集約済み representative view」を、初回 sync command 起点の
  `workflow-step-status-changed` event と live event の同一 ordered チャネルで供給する。frontend は
  version Map を持たない。
- read model は backend-owned で、Tauri / WS / 将来 daemon が同一 shape を読める形にする。
  full-retention / frontend 再計算経路を増やさない。
- 観測可能な振る舞い（streaming 最終表示・turn phase・permission pending・queue・step status）は不変。

## Open Questions

なし。
