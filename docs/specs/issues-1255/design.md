# Design

requirements.md / behavior.md で確定した「Stop を turn のインタラプト境界として扱う」要求を、Claude backend（`claude-sdk-bridge`）の実装方針に落とす。本書は requirements の仮定が design に委ねた以下を確定させる。

- interrupted turn の境界をどこでどう表現するか
- late event（late stream / late result / late turn_complete）の fencing 方式
- 停止済み `currentSessionId` の resume 方針（turn generation / turn token の導入）

---

## 概要

### 現状の不具合連鎖（根本原因）

調査により、Stop からバグ顕在化までの連鎖を以下の通り特定した。ファイルパス・行番号は調査時点のもの。

1. ユーザーが応答中に Stop → フロント `useAgentChat.interrupt`（`src/hooks/useAgentChat.ts:737`）→ `invoke("interrupt_agent_query")` → Rust `interrupt_active_agent_turn`（`src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs:5878`）が bridge stdin に `{"type":"interrupt"}` を書き込む。
2. bridge sidecar（`src-tauri/resources/claude-sdk-bridge.mjs:160`）が `currentAbortController.abort()` を実行。abort で結果（`result`）が出ないまま終わったループは **`turn_complete` を `exit_code: 0` で emit する**（同 `:320-326` / `:349-355`）。**この時点で interrupted turn は正常完了 turn と区別不能になる。**
3. bridge は abort された turn の `currentSessionId`（中断済み SDK session）を保持し、次ループで `options.resume = currentSessionId` として再開する（同 `:264-266`）。
4. Rust `turn_complete` ハンドラ（`bridge_common.rs:3706`）は `effect.was_streaming && exit_code == 0` を成功完了とみなし、`persist_agent_session_id` で**中断済み session id を resume ポイントとして永続化**。turn_phase → `Idle`、state → `Ready` に遷移。
5. `emit_session_state_changed(Idle, Some(0))`（同 `:984`）→ フロント `useAgentSdkListeners`（`src/hooks/useAgentSdkListeners.ts:456`）が `MARK_AGENT_TURN_COMPLETED` + SessionState `done` を dispatch。UI は completed 扱いで送信ボタンへ戻る。
6. ユーザーが新メッセージ送信 → 中断済み `currentSessionId` を resume → SDK が中断 turn の未完了 tail を継続 → **長時間待機の末、新入力ではなく Stop 前 turn の続きを返す。**

### 修正方針

Stop を「provider abort」から「turn のインタラプト境界」へ格上げするため、**interrupted を bridge → Rust → フロントへ一貫して伝播する第一級シグナル**として導入し、以下を成立させる。

- **A. interrupted の真正シグナル化**: bridge が abort 由来の `turn_complete` に `interrupted: true` を付与する（`exit_code` の値に依存しない明示フラグ）。
- **B. resume rollback**: interrupted 時、次 turn が中断 tail を継続しないよう、resume ポイントを「最後に `result` を出した正常完了 turn の session id」へ巻き戻す（bridge 内）と同時に、Rust が interrupted turn の session id を永続化しない。
- **C. late event fencing**: Rust 発行の `turn_token` を `message` コマンドへ載せ、bridge が `turn_complete` / stream にエコーバックする。Rust は active turn の token と一致しないイベントを破棄する。
- **D. completed 誤認の排除**: interrupted フラグを `agent-session-state-changed` ペイロードへ追加し、フロント・SessionStore・AgentStatusCenter で `completed` と区別する。

### 参照: OpenCode の実装方針

設計確定にあたり OpenCode（`packages/opencode`）の中断処理を調査し、以下を裏付けとした。

- **中断は専用 status enum 値ではなく専用マーカーで表現する**: OpenCode は `status:"aborted"` のような enum 値を持たず、message に `AbortedError`（`"MessageAbortedError"`）という専用エラー型 + `time.completed` をセットして中断を表す。UI では `AbortedError.isInstance()` を判定し、通常エラーとも完了とも別扱いにする（`session/prompt.ts:1256` `finalizeInterruptedAssistant`、`session.ts:37` `AbortedError`）。→ 本設計の AgentState 表現を **(c)（新 enum 値を足さず独立フラグで区別）** に確定する根拠（後述）。
- **late event fencing に seq を用いる**: `run-coordinator.ts:193` の `interruptSeq` が「古い wake を抑制し、seq の新しいものだけ通す」方式を採る。→ 本設計の `turn_token` fencing（C）と同型であり方針を裏付ける。
- **中断 turn は履歴に残す**: OpenCode は中断 message を履歴に残し、次 turn を新規に開始する。ただし OpenCode は会話履歴を自前で provider へ渡すため、SDK 側 resume が中断 turn を継続する問題が構造的に起きない。releash は Claude Agent SDK の `resume` が SDK 内 transcript を継続するため、同じ「履歴保持」を素朴に採ると本不具合が再発する。この差分が resume 方針（B）を releash 固有の判断にする（リスクと代替案参照）。

---

## 変更対象

| レイヤー | ファイル | 変更概要 |
|---|---|---|
| bridge sidecar | `src-tauri/resources/claude-sdk-bridge.mjs` | abort 由来 `turn_complete` に `interrupted: true` 付与 / resume rollback / `turn_token` エコー |
| bridge sidecar util | `src-tauri/resources/bridge-utils.mjs` | abort 後の `turn_complete` 生成・resume 巻き戻し判定を純関数化（テスト容易化） |
| Rust runtime | `src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs` | `turn_complete` の interrupted 分岐 / `persist_agent_session_id` 抑止 / `turn_token` fencing / `emit_session_state_changed` への interrupted 伝播 |
| Rust usecase | `src-tauri/src/usecase/workflow/turn_complete.rs`（および `WorkflowTurnCompleteCommand`） | workflow 通知へ interrupted を伝播 |
| Rust status | `src-tauri/src/usecase/agent_session/status.rs` / `src-tauri/src/protocol/agent.rs` | AgentState の interrupted 表現（Open Questions 参照） |
| フロント | `src/hooks/useAgentSdkListeners.ts` | interrupted 受信時に `MARK_AGENT_TURN_COMPLETED` / `done` 化を抑止 |
| フロント | `src/hooks/agentChatReducer.ts` | interrupted 状態の保持・表示用導出 |
| フロント | `src/types/`（`SessionStateChanged` 型） | ペイロードに `interrupted` 追加 |

非対象（requirements の非スコープに従う）: Codex 等他 backend の interrupt 経路、Stop ボタン UI、stale timeout / 通常完了 / workflow step の既存仕様。

---

## アーキテクチャと責務分割

`rust-first-logic` に従い、interrupted 境界の判定・fencing・resume 方針はすべて bridge（Node sidecar）と Rust に置く。フロントは `interrupted` フラグを受けて表示状態を分岐するだけに徹する。

```
[Front]                [Rust runtime]                 [bridge sidecar (Node)]            [Claude SDK]
 Stop ──invoke──▶ interrupt_active_agent_turn ──stdin {"type":"interrupt"}──▶ abort() ──▶ AbortController
                                                                                  │
                 turn_complete{interrupted,turn_token} ◀── stdout turn_complete ──┘ (resume rollback)
                       │
   ┌── interrupted? ──┤
   │ yes              │ no
   ▼                  ▼
 抑止:               従来:
 - MARK_COMPLETED   - persist_agent_session_id
 - done 化           - SessionState=done
 表示: 中断境界       - turn_complete 通知
```

### 各層の責務

- **bridge sidecar**: turn のインタラプトを唯一知る層。abort 検知 → `interrupted: true` 付与、resume ポイント巻き戻し、`turn_token` エコー。
- **Rust runtime（bridge_common）**: interrupted の中継と fencing の中枢。interrupted turn では (1) session id を永続化しない、(2) `turn_token` 不一致イベントを破棄、(3) `emit_session_state_changed` に interrupted を載せる、(4) workflow 通知へ interrupted を伝播。pending queue の drain 自体は従来どおり行う（Stop 後にキューされた新メッセージは新 turn として開始されるべきため）。
- **Rust usecase / status**: interrupted を completed と区別した状態として集約。
- **フロント**: interrupted を受けて `idle`（送信ボタン）には戻すが `done`/completed としては扱わない。

---

## データモデルまたは型

### 1. bridge `turn_complete` メッセージ（拡張）

```jsonc
{
  "type": "turn_complete",
  "session_id": "<sdk session id or null>",
  "exit_code": 0,
  "interrupted": true,        // 追加: abort 由来の中断 turn のみ true
  "turn_token": "<echo>"      // 追加: message コマンドで受け取った token をエコー
}
```

- `interrupted` は **abort 経路でのみ** `true`。`result` を伴う自然完了・`exit_code:1` の自然エラー・stale timeout 経路は `false`（省略時は `false` 扱い）。
- 既存の `exit_code` セマンティクスは変えない（中断は `exit_code:0` のまま）。判定は `interrupted` フラグを正とする。

### 2. bridge `message` コマンド（拡張）

```jsonc
{ "type": "message", "prompt": "...", "images": [...], "turn_token": "<rust-issued>" }
```

bridge は受信時に `currentTurnToken = cmd.turn_token` を保持し、当該 turn の `turn_complete` / stream 系 emit にエコーする。

### 3. `turn_token` の生成（Rust）

- 既存の **agent message id**（turn ごとに新規採番される、`PreparedAgentTurn` / pending drain 時に生成）を `turn_token` として再利用する。新規 ID 体系は導入しない。
- `AgentProcess` に `active_turn_token: Option<String>` を追加し、turn 開始時（`begin_turn_liveness` 近傍）にセット、turn_complete 確定時にクリアする。`turn_seq`（`bridge_common.rs:604`）は watchdog fencing 用に既存のまま併存させ、`turn_token` は late event（特に turn_complete）の同一性照合に用いる。

### 4. `agent-session-state-changed` ペイロード（拡張）

```jsonc
{ "chat_session_id": "...", "turn_phase": "idle", "exit_code": 0, "completed_at": 123, "interrupted": true }
```

- フロント型 `SessionStateChanged`（`src/types/`）へ `interrupted?: boolean` を追加。

### 5. SessionState / AgentState のマッピング

- Rust `SessionState`（`usecase/agent_session/session/mod.rs:163`、`Active|Idle|Done|Error|Closed|Archived`）: interrupted turn → **`Idle`**（成功の `Done` でも `Error` でもない、入力待ち状態）。
- フロント SessionState: interrupted → `idle`（`done`/`error` にしない）。
- AgentState（`usecase/agent_session/status.rs:8`、`Running|Done|Error|Waiting`）: **新 variant を追加しない**。interrupted 時は `AgentStateSync`（worktree 単位 broadcast）を `Done` で発火**しない**。区別は session 単位の `agent-session-state-changed` の `interrupted` フラグで表現し、AgentStatusCenter はこのフラグで中断を分岐表示する。OpenCode が「中断専用の status enum 値を持たず専用マーカーで区別する」設計を採っていることに倣い、protocol への enum 追加（および WS クライアント deserialize 後方互換リスク）を回避する。

---

## 処理フロー

### F1. Stop（応答中 → interrupt）

1. フロント Stop → `interrupt_agent_query` → `interrupt_active_agent_turn` が `{"type":"interrupt"}` を bridge へ。
2. bridge `abort()`。abort 検知ループが `turn_complete{interrupted:true, exit_code:0, turn_token}` を emit し、`currentSessionId` を最後の `result` 時点（`lastResultSessionId`）へ巻き戻す。
3. Rust `turn_complete` ハンドラ:
   - `interrupted == true` の場合、`persist_agent_session_id` を **呼ばない**（中断 session を resume ポイントにしない）。
   - `run_turn_complete_transition_locked` で turn_phase → `Idle` へ（UI は送信ボタンへ戻る）。
   - `emit_session_state_changed(Idle, exit_code, interrupted=true)` を発火。
   - workflow 通知（`spawn_workflow_turn_complete_notification`）へ interrupted を伝播。
4. フロント `useAgentSdkListeners`: `interrupted==true` のとき `turn_phase` のみ更新し、`MARK_AGENT_TURN_COMPLETED` と SessionState `done` 化は行わない。SessionState は `idle`。

### F2. Stop 後の次送信（送信ボタン状態 → 新 turn）

1. ユーザー送信。session は busy ではないため新 turn 経路（`bridge_common.rs:7619` 系）で human/agent message を生成、新 `turn_token`（= 新 agent message id）を採番し `message` コマンドへ載せる。
2. bridge は `currentSessionId == lastResultSessionId`（中断 tail を含まない）から resume するため、**新入力に対する応答として turn を開始**。長時間待機は発生しない。

### F3. Stop（interrupt 進行中）に重ねた送信（pending queue 経路）

1. interrupt 飛行中に送信 → busy 判定で pending queue に積まれ（`bridge_common.rs:7583`）、`interrupt_active_agent_turn` が発火（同 `:7615`）。
2. interrupted `turn_complete` 到着 → 従来どおり pending を drain し、**新 turn として** `start_pending_message_turn` で開始。drain される新 turn には新 `turn_token` を採番。
3. workflow 通知は interrupted フラグ付きで送られ、completed turn と区別される。

### F4. late event fencing

- **late turn_complete**: 古い `turn_token` を持つ `turn_complete` は、active turn の `turn_token` と不一致なら破棄する。これにより「新 turn 応答中に遅延 turn_complete 到着 → 新 turn が誤って完了扱い」を防ぐ（behavior の該当 Scenario）。
- **late stream / late result**: stream 系イベントも `turn_token` を照合し、active でない turn の delta は新 turn の agent message に flush しない。message id がそもそも turn ごとに異なるため、古い delta は古い（中断）message を更新するに留まり、新 message には混入しない（多重防御）。

### F5. 既存経路（非回帰）

- 通常完了（`result` あり、`interrupted=false`）: `persist_agent_session_id` → `Done` の従来挙動を維持。
- stale timeout（`finalize_turn_as_timeout_locked` / `STALE_EXIT_CODE`、`bridge_common.rs:1459`）: `interrupted=false` のため従来どおり。
- workflow step 実行: `interrupted=false` の通常 turn_complete として従来どおり。

---

## エラー処理

- **interrupt 書き込み失敗**: 既存どおりフロントの楽観 `interrupting` フラグを戻す（`useAgentChat.ts:737` 周辺の既存処理を維持）。
- **`interrupted` フラグ欠落（後方互換）**: フィールド未存在時は `false` 扱い。古い bridge と新 Rust の組合せでは従来挙動に縮退する（安全側）。
- **`turn_token` 欠落**: token 不在の `turn_complete` は「token 照合をスキップして従来どおり処理」する（新規導入による既存経路の破壊を避けるフォールバック）。fencing は token を持つ新経路でのみ強制する。
- **resume rollback 先が無い（初回 turn を中断）**: `lastResultSessionId` が未確定なら `currentSessionId` を `null`（新規 session）へ。中断 tail の継続を確実に避ける。
- **interrupted turn の session id 永続化抑止に伴う context 復元**: 既存の `session_ready` resume mismatch 経路（`bridge_common.rs:3539` 周辺）は変更しない。interrupted では永続値を更新しないだけで、最後の正常完了 session id が resume ポイントとして残る。

---

## テスト方針

### Rust 単体（`bridge_common.rs` の `#[cfg(test)]`）

- interrupted `turn_complete` で `persist_agent_session_id` が呼ばれないこと。
- interrupted `turn_complete` で `emit_session_state_changed` の interrupted フラグが立ち、SessionState が `Idle`（`Done` でない）こと。
- `turn_token` 不一致の late `turn_complete` が破棄され、新 turn を完了扱いにしないこと（behavior: 遅延 turn_complete）。
- `turn_token` 不一致の late stream/result が active turn の message に混入しないこと（behavior: late stream/result）。
- interrupt 進行中送信 → pending drain で新 turn が開始され、workflow 通知に interrupted が伝播すること。
- 非回帰: 通常完了で従来どおり永続化＋`Done`、stale timeout（`STALE_EXIT_CODE`）が `interrupted=false` で従来挙動、workflow step が従来挙動。

### bridge sidecar（`bridge-utils.test.mjs`）

- abort 後の `turn_complete` 生成が `interrupted:true` + `turn_token` エコーになること（純関数化した生成ロジックを対象）。
- resume 巻き戻し判定: 中断時に resume ポイントが `lastResultSessionId` になること、初回中断時に `null` になること。

### フロント（Vitest）

- `useAgentSdkListeners.test.ts`: interrupted イベントで `MARK_AGENT_TURN_COMPLETED` が dispatch されず、SessionState が `done` 化しないこと。turn_phase は `idle` に戻ること。
- `agentChatReducer.test.ts`: interrupted 状態が completed と区別して保持・導出されること。

---

## リスクと代替案

- **resume rollback による中断 turn の履歴扱い**: 採用案は「中断 turn のユーザー入力＋部分出力を resume 履歴から落とす」。これにより「次送信は新入力への応答」を満たすが、会話履歴上は中断 turn が消える。OpenCode は逆に中断 message を履歴に残す（`AbortedError` マーカー付き）が、これは OpenCode が会話履歴を自前で provider へ渡し SDK-resume 継続が起きない構造だから成立する。releash は Claude Agent SDK の `resume` が中断 transcript を継続するため、履歴保持を素朴に採ると本不具合が再発する。**実装時に「Claude Agent SDK が abort 後に transcript をどう finalize するか」を実機確認すること**（リスク）。もし SDK が中断 turn を「停止済み」として clean に finalize できると確認できれば、OpenCode 同様に履歴保持（rollback なし）へ切り替える余地がある（代替案）。確認が取れるまでは、新入力への応答開始の確実性を優先し rollback を採用する。
- **`turn_token` エコーの SDK 非対応**: token は bridge 自身が `turn_complete`/stream emit に付与するため SDK 側の対応は不要。SDK の `result` メッセージ自体には付かないが、bridge が turn 境界を握っているため問題ない。
- **late event の実順序**: bridge は単一プロセスで abort → ループ `continue` → 次 query を直列化するため、現状でも late turn_complete のレースは限定的。ただし behavior が明示要求するため `turn_token` fencing を多重防御として実装する。
- **AgentState protocol 拡張の影響**: 新 variant 追加は WS クライアントの deserialize に影響しうる（Open Questions 参照）。

---

## 仮定

- 対象 backend は Claude backend（`claude-sdk-bridge`）に限定（requirements/behavior の仮定を継承）。
- `turn_token` には既存の agent message id を再利用する（turn ごとに新規採番される既存資産）。新 ID 体系は導入しない。
- interrupted turn の SessionState は `Idle`、SDK session の resume ポイントは「最後の正常完了 turn」とする。
- `interrupted` / `turn_token` フィールドが欠落する組合せでは従来挙動に安全に縮退する（後方互換フォールバックを設ける）。

---

## Open Questions

なし。

（AgentState における interrupted の表現は、OpenCode が中断専用の status enum 値を持たず専用マーカーで区別している実装を確認したうえで、option (c)＝新 enum 値を追加せず session 単位の `interrupted` フラグで区別する方針に確定した。データモデル §5 参照。）
