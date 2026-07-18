# requirements.md — S10a: エラーの live 着地（crash / Fatal の即時可視化）

- Issue: #1398
- Milestone: 84「Agentチャット安定化」／ Phase 0（依存なし・即着手可）
- 解消する監査項目: FE-2（high）, RT-6（low）
- 正本:
  - 問題詳細: `specs/milestone-84-agent-chat-stabilization/agent-chat-instability-audit.md`（FE-2 / RT-6）
  - ライフサイクル: `specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-lifecycle.md` I12（エラーの着地保証）
  - 表示: `specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-presentation.md`「turn 終端の表示」・マトリクス session 状態行

## 背景と目的

Agent チャットでは、turn 実行中に CLI プロセスが死ぬ（crash）場合と、Idle 中に backend プロセス（特に常駐する Codex app-server）が死ぬ（Fatal）場合に、エラーが live UI に着地しない。

- **FE-2（high）**: turn の crash エラーは durable log には着地する（`finalize_turn` が `ToolCallFailed` と `TurnInterrupted{error}` を追記し、projector が Error part を合成する）が、live へは emit されない。live 経路は `agent-session-state-changed` のみで、frontend はそれを Error 表示へ変換しない。結果、生成中に spinner が消えて「agent が勝手にやめた」ように見え、reload（= `get_session` 再読込）後にだけ Error block と失敗 tool 結果が現れる。
- **RT-6（low）**: `AgentRuntimeEvent::Fatal` が phase==Idle で発生した場合、`should_complete_crash=false` のため `log::warn` と `set_session_state(Error)` と transient な state change emit のみで、durable event も Error part も残らない。state change payload に message フィールドがないため理由は live 通知にも含まれず、`SessionState::Error` は永続化されるが理由は不明のまま。さらに次の event append で projection が state を上書きし、Error だった痕跡自体が消える。

目的は、backend プロセス死・turn 失敗・Fatal が **発生時点で live UI へ即時着地し、かつ reload 後の表示と一致する**ことを保証すること（ideal-lifecycle I12「エラーの着地保証」の実現）。これにより「live と reload 後で見え方が違う」問題クラスを、当該 2 経路について解消する。

## スコープ

1. **live 着地（FE-2）**
   - `runtime/usecase.rs` の crash finalize 経路（`complete_turn` / `TurnResult::Interrupted{Crash}`）で durable 化された Error part / 中断情報が、transient emit（streaming delta / state change）でも即時 frontend へ届くことを保証する。
   - 監査 FE-2 が特定した「durable には書くが live emit しない」経路に emit を追加する。
   - frontend が受信した crash/中断情報を chat panel の Error 表示へ変換する（現状 `agent-session-state-changed` handler は SET_ERROR 等を dispatch せず、chat panel に session.state を描画するコードも無い）。
   - live と reload 後の表示が一致することをテストで固定する。

2. **Idle 中 Fatal（RT-6）**
   - `apply_runtime_event` の Fatal 分岐（Idle 時）で、エラーメッセージを durable（現行 Error part / 既存語彙の記録先）に書く。
   - session state に「Error の理由」を保持し、read model / `get_session` から復元可能にする。
   - 次の event append による projection 上書きで Error の理由が消えないようにする。
   - Idle-Fatal の理由を **chat panel 内の Error block としても live 着地**させ、reload 後の表示と一致させる。active turn / message が無い状態での Error part の紐付け先は design node で確定する。

3. **badge 理由表示**
   - session バッジ（一覧・タブ）の Error 状態に理由 tooltip を表示する（presentation マトリクス session 状態行）。
   - reload 後も理由が残る（durable 由来）。

## 非スコープ

- **語彙拡張をしない**。現行の Error part / session state で実装し、`Notice(kind)` への移行は S5 で行う。
- **timeout 経路の対応をしない**。監査の訂正どおり `InterruptReason::Timeout` は現コードに生成元が無く、live 経路は crash と Fatal のみ（`usecase.rs` の timeout マッピングは残骸）。
- FE-2 と同じ「backend / frontend の経路差」に起因する他の監査項目（FE-1 permission dialog、FE-3 streaming hydration 欠落、FE-5 グローバル error banner 等）は対象外。
- RT-7（キュー停止）、RT-8（final parts 上書き）等、他の RT 項目は対象外。
- workflow への構造化失敗理由（`TurnError`）伝達（RT-5 / I12 後段）は対象外。
- frontend 状態機械（`useAgentChat` / `agentChatReducer` / `useAgentSdkListeners`）の全面的な backend-owned state 化（FE-0 相当の構造リファクタ）は対象外。本 Issue のスコープ内で触れる範囲に限定する。

## 要求事項

- R1: turn 実行中に CLI プロセスが死んだとき、reload せずにその場（live）で Error 表示が chat panel に現れる。
- R2: Idle 中に backend プロセスが死んだとき、その理由が durable に記録され、badge / セッションから読め、かつ chat panel 内に Error block として live 着地する。
- R3: crash / Idle-Fatal いずれのエラーも、live 表示と reload 後（`get_session` 再読込後）の表示が一致する。
- R4: Idle-Fatal 記録後に次の event が append されても、Error の理由が projection 上書きで消えない。
- R5: session バッジの Error 状態は理由 tooltip を持ち、reload 後も理由が残る。
- R6: 実装は現行の Error part / session state の語彙に閉じ、新規語彙（Notice 等）を導入しない。
- R7: ロジックは Rust（usecase / read model / projector）に置き、frontend は受信データの表示に徹する。
- R8: live 着地・durable 記録・reload 一致を Rust 側テストで固定し、frontend の Error 表示変換をテストで固定する。

## 受け入れ基準の概要

- AC1: CLI プロセスを kill すると、その場に Error block が出る（reload 不要）。
- AC2: Idle 中のプロセス死が、理由付きで badge / セッションから読め、その場（live）で chat panel に Error block として現れる。
- AC3: live と reload 後の表示が一致する。
- AC4: Idle-Fatal 記録後に別の event を append しても、Error の理由が消えない。
- AC5: crash 経路・Idle-Fatal 経路の live emit と durable 記録が Rust テストで固定される。

## 仮定

- A1: spec-id は既存慣例（`feat-issues-1374`, `feat-issues-1301` 等）に合わせ `feat-issues-1398` とする。
- A2: crash 時の live 着地は、既存の transient 経路（streaming delta / `agent-session-state-changed`）を拡張して行い、新規 event / DTO 種別の追加は最小限に留める。durable 側の Error part 合成（`finalize_turn` → projector）は既存挙動を維持し、live へ同一情報を届けることを主眼とする。
- A3: RT-6 の理由保持は、`SessionState::Error` に付随する理由を read model / `get_session` から復元できる形で持たせる。projection 上書き対策は、理由を durable event 由来にして再投影で復元可能にすることで満たす。
- A4: badge 理由 tooltip は frontend の表示のみを担い、理由の source of truth は backend read model（`get_session` / session summary）に置く。
- A5: 「live と reload 後の表示が一致」は crash（turn 中）と Fatal（Idle）の両経路を対象とし、両者を自動テストで固定する。

## Open Questions

なし（RT-6 の着地先は「chat Error block も着地」で確定。active turn / message が無い状態での Error part 紐付け方式は design node で具体化する）。
