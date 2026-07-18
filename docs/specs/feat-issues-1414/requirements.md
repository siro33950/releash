# Requirements

## 背景と目的

milestone 84「Agentチャット安定化」／ Phase 0（依存なし・即着手可）で解消する問題 **FE-5** を対象とする。

現状の Agent チャットでは、error banner が session を跨いだ単一のグローバル値として保持されている。このため次の 2 つのユーザー可視の不具合が起きる。

- 送信失敗などの赤 banner が、裏で動いている別 session が次の turn を始めた瞬間に、ユーザー操作なしで勝手に消える。
- 別 session で発生したエラーが、いま見ている session のパネルに表示される（混線）。

具体的には、`AgentChatState.error` は単一のグローバルフィールドであり（`src/hooks/agentChatReducer.ts:40`）、`BoundSessionChat` は context の `error` をそのまま表示対象 session の `ChatSessionView` に渡す（`BoundSessionChat.tsx`）。`WorkflowView` は workflow node ごとの pane grid で複数の `BoundSessionChat` を同時 mount するため、session A の banner が同時表示中の session B の pane にも出る。さらに reducer の `upsertSession` は無条件に `error: null` を返し（`agentChatReducer.ts:315`）、`useAgentSdkListeners.ts` の `agent-turn-prepared` listener は worktreePath 一致のみで `UPSERT_SESSION` を dispatch するため、同一 worktree 内の別 session の turn 開始（backend の queued turn drain による `turn_prepared` emit）や `loadSession` 成功が起きた瞬間に、表示中の banner が無言でクリアされる。

本変更の目的は、error banner を session スコープの一時通知として正しく閉じ込め、他 session の活動によって banner が消える・混ざることをなくすことである。正本ドキュメント（[agent-chat-ideal-presentation.md](../../../specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-presentation.md) 「エラー・バナーのスコープ規則」・P4）の規則に一致させる。

正本ドキュメント:

- 問題詳細: `specs/milestone-84-agent-chat-stabilization/agent-chat-instability-audit.md`（FE-5）
- 表示規則: `specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-presentation.md`（「エラー・バナーのスコープ規則」・P4）

## スコープ

- error banner state を Rust usecase が session_id をキーに保持し、frontend は表示に必要な snapshot のみを mirror する。
- banner の表示を、対象 session のパネル（`BoundSessionChat` / `ChatSessionView`）にのみ行うようにする。他 session の pane に混ざらないようにする。
- banner のクリア規則を、presentation「スコープ規則」に一致させる。無言クリアの分岐（`UPSERT_SESSION` 等の無関係イベントによる `error: null`）を廃止し、クリア契機を次の 2 つに限定する。
  1. ユーザーの明示 dismiss。
  2. 同一 session における対象操作の成功（例: 送信失敗後に同一 session で再送信が成功）。
- 上記の振る舞いを固定する component / reducer テストの追加・更新。少なくとも次を pin する。
  - session A でエラー → session B が turn 開始 → A の banner が残る。
  - session B のエラーが A のパネルに出ない。
- `agentChatReducer.test.ts` の既存「UPSERT_SESSION clears error」テストを、新しいスコープ規則に沿って更新する。

## 非スコープ

- turn に紐づくエラーの表示（S1 の durable part を正本とする経路）は変更しない。banner は「操作の失敗（送信・切替等）」という session スコープの一時通知に限定し、turn error の表示器にはしない。
- app スコープの通知（更新通知等）は対象外。
- FE-5 以外の milestone 84 問題（FE-1〜FE-4、FE-6、FE-7、RG-*、CX-* 等）は対象外。
- error メッセージ文面の変更や、新しいエラー種別の追加は行わない。

## 要求事項

1. **banner state を session スコープで保持する。** Rust usecase は error banner を session_id をキーに保持し、frontend はその snapshot を session_id 別に mirror する。ある session の banner が他 session の状態・イベントの影響を受けないようにする。

2. **表示スコープを一致させる。** banner は対象 session の surface（`BoundSessionChat` / `ChatSessionView`）にのみ表示する。複数 session を同時 mount する `WorkflowView` の pane grid でも、banner が表示対象外の session の pane に現れないようにする。

3. **無言クリアを廃止する。** 他 session の turn 開始（`turn_prepared` 由来の `UPSERT_SESSION`）、`loadSession` 成功、その他の無関係イベントによって banner を無言でクリアしない。`upsertSession` の無条件 `error: null` 分岐を廃止する。

4. **クリア契機を限定する。** banner のクリアは (a) ユーザーの明示 dismiss、(b) 同一 session における対象操作の成功、(c) session 除去、のみとする。操作分類と同種成功の判定は Rust が所有する。

5. **明示的な対象 session を持つ error 設定契機を維持する。** 既存 session の送信失敗・読み込み失敗・クローズ失敗等は、対象 session に紐づく形で引き続き表示できる。list / init / create / `sendMessage(null)` のような session 非依存・生成前処理は、偶然 active な session の banner に変換しない。

6. **テストで振る舞いを固定する。** cross-session の非干渉（残存・非混線）と、クリア契機の限定を、component / reducer テストで pin する。

## 受け入れ基準の概要

- 他 session の活動（turn 開始、`loadSession` 成功等）によって、表示中の banner が消えない・混ざらない。
- 明示 dismiss と、同一 session での対象操作の成功、のみで banner が消える。
- session A で発生した banner が、同時表示中の session B の pane に表示されない。
- 明示的な対象 session を持つ各操作失敗時に、その session のパネルに banner が表示される。
- background refresh や session 生成前の失敗が、active session の banner に混線しない。
- 上記を固定する component / reducer テストが追加・更新され、既存テストは新スコープ規則に沿って整合している。
- 受け入れ基準（Issue 記載）:
  - [ ] 他 session の活動で banner が消えない・混ざらない。
  - [ ] 明示 dismiss と成功時クリアのみで banner が消える。

## 仮定

- **Spec ディレクトリ ID** は worktree 名に合わせ `docs/specs/feat-issues-1414/` とする（既存の `feat-issues-1374` 等と同一命名規則）。
- **操作分類・自己回復 policy・notice state は Rust usecase が所有する。** frontend は command 結果の snapshot mirror、session 別 selector、描画、dismiss 入力のみを担当する。
- **クリア判定の「対象操作の成功」の粒度**は、banner を発生させた操作と同種の操作が同一 session で成功したときとする（例: 送信失敗 banner は同一 session の送信成功でクリア）。厳密な操作対応表は behavior / design で確定する。
- **session がストアから除去される（close / remove 等）際は、その session_id の banner エントリも破棄する。** full-retention を避けるため、閉じた session の banner を残さない。
- **session をまだ持たない処理は session banner の対象外とする。** list / init / create / `sendMessage(null)` の失敗は active session に anchor しない。作成要求のエラーは既存の呼び出し元 surface へ返す。
- banner の UI 見た目・文面・dismiss 操作の導線は現状を踏襲し、変更しない（スコープ化とクリア規則のみを変える）。

## Open Questions

なし
