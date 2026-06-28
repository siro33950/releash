# Requirements

## Type

実装 ISSUE（リファクタリング / アーキテクチャ移行）。親 ISSUE ではない。

マイルストーン: [12] クリーンアーキテクチャ移行。

関連:

- Depends on: #1130（protocol consolidation・CLOSED） / #1131（ws_server / ws_bridge migration・CLOSED）
- Blocks: #878（final dead-code sweep・OPEN）
- 親方針: `.claude/rules/rust-first-logic.md`（全アプリケーションロジックは Rust に置き、frontend はインターフェースに徹する）

## 背景と目的

CLAUDE.md / `rust-first-logic.md` の大方針に従い、agent session / stream / status の application state 更新を frontend reducer / parser から Rust-owned session read model へ移す。frontend は backend-owned session state の mirror と描画に限定し、session domain の source of truth を frontend reducer から外すことが本 ISSUE の目的である。

依存先（#1130 / #1131）は完了済みであり、protocol DTO / stream message shape（#1130）および WebSocket / app event / resync / replay 境界（#1131）は確定している。本 ISSUE はその確定境界の上で、frontend に残る stream parse / 順序付け / 状態集約の domain decision を解消する。

### 現状のコード調査（事実）

ISSUE 本文の「対象コード」「やること」は計画時点の想定であり、実コード調査の結果、以下の点で実態が異なることが判明した。本 requirements は実コードに合わせて記述する。

#### ① raw stream-json parser / ANSI 処理は production 未消費の dead code

- `src/lib/parseStreamJson.ts`（`formatEvent` / `formatCodexItem` / `formatAssistant` / `formatToolUse` を含む stream-json NDJSON → 表示テキスト変換）を import している production コードは存在しない。参照は `src/lib/parseStreamJson.test.ts` のみ。
- `src/lib/stripAnsi.ts`（ANSI escape / control char 除去・carriage return 処理）の production 参照は `parseStreamJson.ts`（それ自体が dead）と自身の test のみ。
- `src/components/panels/AgentChatPanel/ActivityLog.tsx` は `parseStreamJson` / `stripAnsi` を import・呼び出していない。ActivityLog は backend が返すツール出力（`getSessionToolOutput` 等）と構造化済み message part を描画しており、frontend で raw stream-json を parse する経路は存在しない。
- すなわち「raw stream-json を frontend で parse しない経路にする」は、新規移行というより **既存の未消費 parser を削除して、frontend に raw parser が残っていない状態を確定させる** 作業である（#1303 で確認された「ISSUE が live と想定したコードが実際には dead code だった」のと同型）。

#### ② worktree / session 状態集約ユーティリティも production 未消費の dead code

- `src/lib/agentStateUtils.ts`（`aggregateAgentState` / `highestPriorityState` / `aggregateFromEntries` / `agentStateKey`。worktree path ごとに agent state を優先度集約するロジック）を import している production コードは存在しない。参照は `src/lib/agentStateUtils.test.ts` のみ。
- worktree ごとの agent/session status aggregation を「frontend から Rust へ移す」とあるが、対象の `agentStateUtils` は現状 production から消費されていない。

#### ③ 生きた stream / session state の source of truth は reducer + listener にある

ISSUE は対象を `src/hooks/useAgentChat.ts` の stream/session 更新箇所と記すが、実際に streaming delta の順序・重複・drop・resync を決定している live コードは次の 2 箇所である。

- `src/hooks/agentChatReducer.ts`
  - `APPLY_STREAMING_DELTA`: `lastStreamingSeqByMessage` を保持し、`seq <= lastSeq || seq !== lastSeq + 1` の delta を drop、`seq === lastSeq + 1` のみ適用する **seq 単調性・gap・重複の判定（domain decision）** を持つ。
  - `SET_STREAMING_MESSAGE`: resync snapshot を message に反映し `lastStreamingSeqByMessage` を更新する。
  - その他、`turnPhases` / `pendingPermissions` / `pendingQueues` / `interrupting` 等の session domain state を frontend reducer が保持する。
- `src/hooks/useAgentSdkListeners.ts`（ISSUE の対象コード一覧には未記載だが実体はここ）
  - `agent-streaming-delta` listen 時に seq gap を検出し、gap / 空 parts 時に `resync_streaming_message`（backend command）を呼ぶ **resync オーケストレーション** を持つ。
  - `getEffectiveLastSeq` / `advanceOptimisticLastSeq` による楽観的 last-seq 追跡、`hydrateMessageIfMissing` による欠損 message の再取得を持つ。
  - `agent-session-state-changed` listen 時に `SET_TURN_PHASE` / `MARK_AGENT_TURN_COMPLETED` / `SET_PENDING_PERMISSION` を dispatch する。

#### ④ backend は既に seq / resync / session state を所有している

- streaming delta の `seq` は Rust 側で付与される（`src-tauri/src/adaptor/protocol/agent.rs` の `seq: u64`）。
- `resync_streaming_message`（`src-tauri/src/usecase/agent_session/session/stream_resync.rs`、command 経由で frontend が `resyncStreamingMessage` として呼ぶ）が since_seq 指定の snapshot read model を返す。`streaming_final_seq` も backend で永続化される。
- session state は `src-tauri/src/domain/agent_session` / `src-tauri/src/usecase/agent_session/session` が所有し、`agent-session-state-changed` / `agent-streaming-delta` イベントを発火する。
- すなわち backend は **ordering / resync / session state の正典をすでに持っている** が、frontend reducer + listener が「seq gap を検出して keep/drop を判断し、resync を駆動する」二重判断を残している状態である。

#### ⑤ workflow step status の集約も frontend に残る

- `src/hooks/useWorktreeStepStatuses.ts` は `workflow-step-status-changed` イベントを listen し、`version` ベースの dedup（`stepVersions` / `workflowVersions`）と step / workflow representative の Map 集約を frontend で行う。

## スコープ

本 ISSUE は、上記事実に基づき以下を範囲とする。

- **A. raw parser / ANSI 処理の dead code 削除（①）**
  - `src/lib/parseStreamJson.ts` とそのテストを削除する。
  - `src/lib/stripAnsi.ts` とそのテストを、他に production 消費者がないことを確認した上で削除する。
  - 結果として「frontend に raw stream-json parser / ANSI 整形が残っていない」状態を確定させる。
- **B. worktree / session state 集約ユーティリティの dead code 削除（②）**
  - `src/lib/agentStateUtils.ts` とそのテストを、production 消費者がないことを確認した上で削除する。
  - frontend に worktree 単位の agent state 優先度集約ロジックが残らない状態にする。
- **C. streaming delta の順序 / 重複 / drop / resync の所有を backend へ移し、配信契約ごと backend 主導にする（③④）**
  - **配信契約ごと backend 主導とする（合意済み）。** frontend からの seq gap 検出・`resync_streaming_message` 駆動・楽観的 seq 追跡といった resync オーケストレーション自体を撤去し、backend が gap-free に順序付け・重複排除済みの delta を push する配信契約へ移行する。frontend は backend が確定した順序・内容を反映する純粋 mirror になる。
  - `agentChatReducer` の `APPLY_STREAMING_DELTA` から seq gap / 重複 / drop の判定（`seq <= lastSeq || seq !== lastSeq + 1` による keep/drop、`lastStreamingSeqByMessage` の保持）を除去し、backend が順序保証した delta を素直に適用する形へ縮小する。
  - `useAgentSdkListeners` の gap 検出・`resync_streaming_message` 呼び出し・`getEffectiveLastSeq` / `advanceOptimisticLastSeq` / `hydrateMessageIfMissing` 等の resync オーケストレーションを撤去し、backend-owned ordering / replay を反映する最小の listen 配線へ縮小する。
  - seq ordering / duplicate delta / drop / reconnect replay を検証する test を Rust 側へ持つ（frontend 側の同等 test は invoke/listen 配線・描画の検証に縮小する）。
  - 本変更は #1130 で確定した protocol 型の relocation ではなく、agent stream の **配信契約（push される delta の順序保証・重複排除の所在）** を frontend 二重判断から backend 単一所有へ移すものである。配信契約の変更を伴うことを許容する。
- **D. workflow step status 集約の Rust query 化（⑤）**
  - `useWorktreeStepStatuses` の version dedup / step・workflow 集約を Rust 側 query へ移し、frontend は backend が返す集約済み read model を保持・描画するだけにする。
- frontend が引き続き所有してよい責務:
  - active session id
  - panel の開閉・選択
  - backend read model の描画
  - invoke / listen の配線

> 注: C の到達点は「配信契約ごと backend 主導へ」で合意済み（Open Questions 参照）。frontend の gap 検出・resync オーケストレーションは撤去する。

## 非スコープ

- protocol module relocation（#1130 で完了済み。本 ISSUE では touch しない）。
- WebSocket server / bridge migration（#1131 で完了済み）。
- agent backend provider の機能追加。
- chat UI redesign（ActivityLog / ChatSessionView のレイアウト・操作の再設計）。
- dead-code sweep の最終確定（#878 の担当）。本 ISSUE は対象範囲の dead code 削除に閉じ、リポジトリ全体の sweep は行わない。
- session message 本体の永続化形式・event log projector の変更。
- token usage / model selection / backend selection 等、stream ordering と無関係な reducer state の再設計。

## 要求事項

### raw parser / ANSI（A）

- frontend に raw stream-json parser（`parseStreamJson` 相当の event → 表示テキスト変換）が存在しないこと。
- frontend に ANSI escape / control sequence 整形ロジック（`stripAnsi` 相当）が存在しないこと（production 消費者がない前提で削除）。
- 削除に伴い `pnpm build` / `pnpm lint` / `pnpm test` が壊れないこと。

### worktree / session 集約（B）

- frontend に worktree 単位の agent state 優先度集約（`agentStateUtils` 相当）が存在しないこと。

### streaming delta ordering（C）

- streaming delta の seq 順序・重複・drop の判定が frontend reducer / listener から除去され、backend 単一所有になっていること。
- backend が gap-free に順序付け・重複排除済みの delta を push する配信契約になっており、frontend に seq gap 検出・resync オーケストレーション（`resync_streaming_message` 駆動・楽観的 seq 追跡・欠損 message hydrate）が残らないこと。
- reconnect / replay 時の整合（欠損 delta の補完）が backend-owned で完結し、frontend は backend が確定した順序・内容を反映するだけであること。
- seq ordering / duplicate delta / drop / reconnect replay を検証する test が Rust 側に存在すること。
- frontend reducer が session domain state（少なくとも streaming 適用順序）の source of truth になっていないこと。

### workflow step status 集約（D）

- workflow step status の version dedup / 集約が Rust query 側で行われ、frontend は集約済み read model を保持・描画するだけになること。
- `useWorktreeStepStatuses` に version 比較・representative 選択の domain decision が残らないこと。

### 横断要求

- read model が backend-owned であり、Tauri 以外の将来 client surface（WebSocket / 将来 daemon）からも同じ shape を読める形であること。full-retention / frontend 再計算経路を増やさないこと。
- frontend test は rendering / interaction / invoke・listen 配線を検証する形に寄ること。
- 外部から観測可能な振る舞い（ストリーミング表示の最終結果、turn phase 遷移、permission pending 表示、queue 表示、workflow step status 表示）が不変であること。

### 品質ゲート

- `pnpm lint` / `pnpm test` / `pnpm build` が通ること。
- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通ること。

## 受け入れ基準の概要

ISSUE の「完了条件」を実コードの実態に合わせて受け入れ基準とする。

- frontend grep で raw stream-json parser（`parseStreamJson` / `formatEvent` 等）が存在しないことを確認できる。
- frontend grep で `stripAnsi` / `agentStateUtils` の export と production 参照が存在しないことを確認できる（テストごと削除済み）。
- frontend reducer / listener が streaming delta の seq gap / 重複 / drop / resync の source of truth でない（backend-owned read model を反映する形になっている）。
- Rust test で raw stream parse・ANSI handling・delta ordering・duplicate/drop handling・worktree(step/session) aggregation のうち、本 ISSUE で backend に所有を確定したものが検証されている。
- workflow step status の集約が Rust query 側にあり、`useWorktreeStepStatuses` に version dedup / 集約 decision が残らない。
- frontend test が rendering / interaction / invoke・listen 配線を検証している。
- 観測可能な振る舞い（streaming 表示結果・turn phase・permission pending・queue・step status 表示）が不変である。
- `pnpm lint` / `pnpm test` / `pnpm build` / `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る。

## 仮定

- **（要レビュー）** `parseStreamJson` / `stripAnsi` / `agentStateUtils` は production 未消費の dead code であるため、Rust 側に同等の raw stream parse / ANSI / agent-state aggregation read model を **新設せず削除する**。backend は既に構造化済み message part / ツール出力 / seq 付き delta を返しており、frontend での raw parse / ANSI 整形は不要であることを前提とする。これは #1303 で確立した「production 未消費 dead code は移行せず削除」方針に倣う。
- backend は既に streaming delta の `seq` 付与、`resync_streaming_message` read model、`streaming_final_seq` 永続化、`agent-session-state-changed` / `agent-streaming-delta` 発火を所有している。本 ISSUE は backend に ordering / replay の正典を確定させ、frontend をその mirror に縮小する。配信契約は「backend が gap-free に順序付け・重複排除済みの delta を push する」形へ移す（合意済み）。既存の `resync_streaming_message` 資産は backend 内の replay 実装として再利用しうるが、frontend からの駆動経路は撤去する。配信契約の具体（push payload の順序保証表現、reconnect 時の replay 起点、frontend が保持する最小状態）は design.md で決定する。
- `useWorktreeStepStatuses` の対象は workflow step status であり、agent session stream とは別系統だが、ISSUE の対象コードに明記されているため本 ISSUE のスコープに含める。集約結果の transport 形（Rust query が返す read model shape）の具体は design.md で決定する。
- 削除・縮小する frontend ロジックの単体テストは、Rust 側の同等 test（ordering / duplicate / drop / replay / step aggregation）へ責務移管する。frontend test は描画・interaction・invoke/listen 配線に寄せる。テストの期待値は実装に合わせて緩めず、観測可能な振る舞いを維持する。
- backend read model / query の具体的な module 配置・transport 型・command 名の決定は design.md で行う。本 requirements では所有境界と振る舞い不変性のみを要件とする。

## Open Questions

なし。

### 解決済み

- **streaming delta ordering 所有移管の到達点（スコープ C の深さ）** → **「配信契約ごと backend 主導へ」で確定。** frontend からの seq gap 検出・`resync_streaming_message` 駆動・楽観的 seq 追跡（resync オーケストレーション）を撤去し、backend が gap-free に順序付け・重複排除済みの delta を push する配信契約へ移行する。frontend reducer/listener は backend が確定した順序・内容を反映する純粋 mirror になる。配信プロトコルの具体は design.md で決定する。
