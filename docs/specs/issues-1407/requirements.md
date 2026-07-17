# requirements: L6 resume 回復の統一（BackendSessionCleared 配線）

- Spec ID: issues-1407
- 対応 Issue: #1407 「[Agentチャット安定化] L6: resume 回復の統一（BackendSessionCleared 配線）」
- 位置づけ: milestone 84「Agentチャット安定化」／ Phase 0
- 解消する監査項目: SD-1（high）, OB-8（low）／ライフサイクル I9

## 背景と目的

### 背景

agent session の backend resume 失敗に対する回復経路が backend 間で非対称であり、Codex 側で恒久的にセッションが死ぬ問題がある。

- **SD-1（high）**: resume 失敗の回復経路が非対称。
  - Claude は resume 失敗（CLI が別 session_id で init を返す）を `ResumeOutcome::Mismatch` として event pump に流し、`handle_resume_mismatch`（`runtime/usecase.rs:2083-2133`）が runtime を閉じ、実行中 turn を pending queue に戻し、resume metadata を消去して新規 backend セッションで自動再開する。ただし**文脈が静かに消える**（ユーザーへの通知が無い）。
  - Codex は thread/resume のエラー応答を `AgentRuntimeEvent::BackendSessionCleared + Fatal` に変換する（`codex/convert.rs:59-67`）が、この event は `open()` が起動フェーズで失敗する経路でしか emit されず、events receiver が `open()` 成功後の `take_events` でしか読まれないため、**受信者のいないチャネルごと drop される**。`BackendSessionCleared` を処理して `agent_session_id` を消去するはずの経路（`runtime/usecase.rs:2420-2438`）には決して到達しない。`domain/agent_session/gateway.rs:60-62` の `#[allow(dead_code)]` コメント自体が配線未完了を明記している。
  - 結果として Codex は backend thread が消えると（codex home 変更・rollout ファイル削除・thread GC 等）、死んだ thread id への resume を送信のたびに繰り返して恒久的に失敗し続け、チャットには生の JSON-RPC エラー（例: not found）が error part として毎回表示されるだけで、復旧手段も自動回復もなく二度と会話を継続できない。queued turn 再オープン経路ではエラー表示すら無く log warn のみ。

- **OB-8（low）**: resume mismatch の requeue で `editor_context` が脱落する。
  - `start_turn_for_session` が `current_turn_input` を構築する際、`QueuedTurnInput::new` の `editor_context` 引数に `None` を固定で渡している（`runtime/usecase.rs:1447-1456`。`From<EditorContext> for AgentEditorContext` が未実装のため）。
  - resume mismatch 時は `handle_resume_mismatch` がこの `current_turn_input` を queue 先頭へ戻し、`start_next_queued_turn` が `queued.editor_context` のみから `TurnInput` を再構築するため、リトライされた turn の `editor_context` は `None` になる。
  - 影響: (a) Codex は `editor_context` を additionalContext としてワイヤ送信するため、リトライではアクティブファイル・選択範囲が送られない。(b) リトライ turn の system prompt 再構築からも editor context が消えるため、Claude セッションでも system prompt 経由のエディタ状態が脱落する。`mentions` や `images` は保持されるのに `editor_context` だけ落ちる非対称。

### 目的

- Claude / Codex 双方の resume 失敗（mismatch・thread 消失）を、同一の回復経路（I9）に統一する。
- Codex の backend thread 消失後も、セッションが恒久死せず、次送信で新規 backend セッションを再確立して会話を継続できるようにする。
- 回復が起きたことをユーザーに通知し、「文脈が静かに消える」状態を無くす。
- resume mismatch 直後のリトライ turn で `editor_context` が失われないようにする。

## スコープ

1. **統一回復経路（I9）: `BackendSessionCleared` の配線**
   - dead code の `AgentRuntimeEvent::BackendSessionCleared` を実際の回復経路に配線する。
   - Codex の thread 消失／resume 失敗検知（`codex/session.rs` の resume エラー経路）を、起動フェーズだけでなく確立済みセッションの送信経路でも `BackendSessionCleared` として runtime に到達させる。
   - `BackendSessionCleared` 受信 → runtime が resume metadata をクリア → 新規 establish を再試行 → 成功後にユーザーへ回復通知を出す、という一連の流れを両 backend 共通経路として成立させる。

2. **Claude 側の通知追加**
   - 既存の無言 mismatch 復旧（`handle_resume_mismatch`）に、Codex と同じ回復通知を追加し、「文脈が静かに消える」を無くす。

3. **`editor_context` 保全（OB-8）**
   - `handle_resume_mismatch` の requeue で `current_turn_input` を再構築する際、`editor_context` を `None` 固定にせず、元の `TurnInput` から引き継ぐ（元の `AgentEditorContext` を引き回すか、逆変換を追加して構築時に渡す）。

4. **回復トランザクションの整合性（最終設計ゲート追補 2026-07-15）**
   - resume metadata clear と `BackendSessionRecoveryStarted { recovery_id, old_provider_session_generation, reason }` を同一の local atomic commit で確定し、`recovery_id` を用いて configuration / Goal を回復中として block する。
   - `SessionConfigurationReactivated` は新 provider session generation と consume した observation id を保存し、旧 effective 値を新 session の実効値として流用しない。
   - Goal は `None` / terminal / unchanged / restored のいずれであっても必ず `SessionGoalReactivated`（`GoalReactivationOutcome` 網羅）へ記録する。
   - 最終 `SessionGoalReactivated + BackendSessionRecoveryCompleted` を同じ atomic batch で append して初めて Synced／公開する。
   - Goal restore strategy が `StartsTurn` の場合は、evidence 付き `TurnStarted` も最終 transaction へ含め、early stream を buffer する。結果不明は Goal / turn（または configuration）reconciliation へ送る。

5. **テスト**
   - codex home 差し替え等で backend thread 消失を模擬し、「次の送信でセッションが復活し通知が出る」統合テストを追加する。
   - Claude の resume mismatch 時に通知が出ることを検証する。
   - resume mismatch 直後のリトライ turn で `editor_context` が保全されることを検証する。

## 非スコープ

- SD-1 / OB-8 以外の監査項目（SD-2 以降、OB-8 以外の OB 項目、RT 項目等）の修正。
- resume/回復以外の起動・Stop・permission 経路の挙動変更。
- backend CLI（Claude Code / Codex）側の resume 実装の変更。
- 回復通知の恒久的な UI 表現（アイコン・専用パネル等）の刷新。通知は既存の Notice / Error part の枠組みに載せる。
- MessagePart の domain / usecase 二重定義の解消（G-1 等、別 issue 残債）。
- milestone 84 で並行する他 issue（#1397 / #1449 等）が主管する範囲。本 issue は共有 event 型（`BackendSessionRecovery*` / `Session*Reactivated`）を SD-1 / OB-8 の回復経路で利用・配線する範囲に限る。

## 要求事項

- R1: `AgentRuntimeEvent::BackendSessionCleared` は dead code ではなく、Claude / Codex 双方の resume 失敗回復経路から emit・受信される配線を持つこと。
- R2: Codex の backend thread が確立後に消失しても、次の送信でセッションが Error のまま恒久死せず、新規 backend セッションを再確立して会話を継続できること。
- R3: backend セッションが作り直された場合、ユーザーへ「backend セッションを作り直したため文脈は引き継がれない」旨の通知を出すこと（Notice。通知経路が未成立なら暫定的に Error part で通知）。
- R4: Claude の resume mismatch による無言復旧時にも R3 と同じ通知を出し、文脈が静かに消えないこと。
- R5: resume mismatch の requeue でリトライされる turn は、元の turn の `editor_context` を保持すること（Codex のワイヤ送信・Claude の system prompt 双方でエディタ状態が失われない）。
- R6: 回復は同一 `recovery_id` で相関付けられ、resume metadata clear と回復開始・configuration/Goal 復旧・回復完了が定義された atomic commit 境界で確定され、部分適用状態が公開されないこと。
- R7: Goal は None / terminal / unchanged / restored のいずれの場合も `SessionGoalReactivated` として必ず記録され、旧 effective 値が新 provider session へ無検証で流用されないこと。
- R8: 上記の回復・保全挙動を検証する統合テストが追加されていること。

## 受け入れ基準の概要

- AC1: Codex の backend thread 消失後もセッションが恒久死しない（次送信で復活＋通知が出る）。
- AC2: Claude の resume mismatch 時に回復通知が出る。
- AC3: resume mismatch 直後の turn で `editor_context` が失われない。
- AC4: `BackendSessionCleared` の `#[allow(dead_code)]` が解消され、production 経路から到達可能になっている。
- AC5: 回復トランザクションが `recovery_id` により相関付けられ、`BackendSessionRecoveryStarted` → configuration/Goal reactivation → `BackendSessionRecoveryCompleted` の順で atomic に確定・公開される。
- AC6: 追加した統合テストと既存テストが green（`cargo test`）、`cargo fmt --check` / `cargo clippy -- -D warnings` が通る。

## 仮定

- （仮定）Spec ディレクトリ名は最近の慣習に合わせ `docs/specs/issues-1407/` とする。
- （仮定）本 issue は SD-1 / OB-8 の回復経路成立を主目的とし、共有 event 型（`BackendSessionRecoveryStarted` / `SessionConfigurationReactivated` / `SessionGoalReactivated` / `BackendSessionRecoveryCompleted` / `GoalReactivationOutcome`）は正本 vocabulary の定義（`agent-chat-ideal-vocabulary.md` §該当箇所）に準拠する。型自体の新規追加が #1397 等の主管であっても、本 issue で回復経路に必要な範囲は配線・利用する。
- （仮定）ロジックは全て Rust（usecase / domain / infrastructure）に置く。frontend は通知の表示のみを担い、回復判断やトランザクション制御を持たない。
- （仮定）回復通知の文言は日本語で、S5（通知経路整備）が未了の場合は Error part を暫定手段とする。監査記述の文言「backend セッションを作り直したため文脈は引き継がれません」を基準とする。
- （仮定）「thread 消失の模擬」は外部プロセスを実行せず、codex home / rollout ファイル差し替え等でテスト内から再現する（外部プロセスをテストで実行しない方針に従う）。

## Open Questions

なし
