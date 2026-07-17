# Design

`requirements.md` / `behavior.md` を満たす実装設計。対象は milestone 84 OB-2（stalled turn 中の送信でユーザー入力が失われる不具合）の解消。ideal lifecycle **I6**（送信は「turn 開始 or queue 追加」に収束）と ideal presentation **P5**（入力の保全）を実現する。

## 概要

現状、`send_message` は stall 観測中（`stall_observation_active`）かつ backend が steering 非対応のとき、`add_human_message_internal` に到達する前にエラー `active-turn steering is not available` を返す（`runtime/usecase.rs:293-297`）。Claude / Codex はいずれも `capabilities().steering == false` のため、stalled turn への送信は本番で常にこのエラーになり、メッセージは永続化されず失われる。frontend も送信直後に無条件で入力欄をクリアし（`MessageInput.tsx:455-457`）、`sendMessage` は catch でエラーを swallow する（`useAgentChat.ts:914-919`）ため、入力復元経路が存在しない。

本設計では次の 2 点を変更する。

- **backend（R1・R2・R6）**: stall 判定中の送信も、通常の実行中送信と同一の queue 経路（既存 `pending_queue` / `QueuedTurnInput`）へフォールバックさせる。steer 分岐は「backend が steering に対応している場合のみ」に限定し、非対応時は queue へ積む。`active-turn steering is not available` の早期エラー return を撤廃する。
- **frontend（R3・R4）**: 送信ハンドラが送信 API の完了を待ち、成功時にのみ入力欄・添付・pasted text・mention をクリアする。失敗は入力保全の責務を持つ層（`MessageInput`）まで伝播させ、入力を保持する。

R5（queue チップ表示）は既存の `pendingQueue` レンダリング（`ChatSessionView.tsx:1747`）で満たされるため、backend が queue へ積むようになれば追加実装なしに達成される。

## 変更対象

### backend（Rust）

- `src-tauri/src/usecase/agent_session/runtime/usecase.rs`
  - `send_message`（`274-431`）: steer 分岐のガードとフォールバック分岐の修正。早期エラー return（`293-297`）の撤廃。
  - テスト `test_stale_watchdog_無進捗turnをstall_signalに留めruntimeを閉じない`（`7561-7651`）: 新仕様（stall 中も queue へフォールバック）に合わせて更新／置換（issues-1301 D16/F-2 の反転）。
  - 新規テスト: stall 中送信 → queue 追加 → turn 終了後 drain の固定。

### frontend（TypeScript）

- `src/hooks/useAgentChat.ts`
  - `sendMessage`（`792-927`）: 送信失敗を呼び出し側へ伝播する（catch で swallow せず、成功/失敗を返す）。
- `src/components/panels/AgentChatPanel/MessageInput.tsx`
  - `handleSubmit` / `submitContent`（`436-489`）: `onSend` の完了を待ち、成功時にのみ入力状態をクリアする。
- 中間層（型シグネチャの伝播のみ）
  - `src/components/panels/AgentChatPanel/BoundSessionChat.tsx` `handleSend`（`155-178`）
  - `src/components/panels/AgentChatPanel/ChatSessionView.tsx` `handleComposerSend`（`1126-1137`）

要求外（変更しない）: stall 判定ロジック、queue 永続化（OB-3）、`cancel_queued_turn`、Agent チャット以外の送信経路。

## アーキテクチャと責務分割

Rust-first 原則に従い、「送信を queue へ積むか turn を開始するか」の意思決定は **すべて backend（usecase）** が所有する。frontend は「送信結果に応じて入力欄をクリアするか保持するか」という UI 制御のみを担う。

| 層 | 責務 | 変更 |
|---|---|---|
| `usecase/runtime` | queue 判定・queue 投入・human message 永続化・steer 分岐 | 分岐条件の修正（本設計の中心） |
| `RuntimeSessionState.pending_queue` | pending queue の source of truth | 変更なし（既存経路を再利用） |
| controller（Tauri command） | 入力を usecase 呼び出しへ変換 | 変更なし（`send_message` の戻り値は既存の `SendMessageResponse`） |
| frontend `useAgentChat.sendMessage` | invoke 呼び出しと結果の mirror | 失敗を呼び出し側へ伝播 |
| frontend `MessageInput` | 入力状態の所有・成功時クリア | 完了待ち＋成功時クリアへ変更 |

backend の戻り値 `SendMessageResponse`（`queued_turn` / `pending_queue` を含む）は既存のまま Tauri command から返す。現行 protocol には WebSocket 経由の agent message 送信入口がないため、その inbound surface の追加は本 ISSUE の対象外とする。将来追加する場合も controller に分岐を複製せず、同じ usecase と応答型を再利用できる。

## データモデルまたは型

### backend

新規スキーマは追加しない。既存を再利用する。

- `QueuedTurnInput`（`runtime/queue.rs`）: queue entry。`existing_human_message_id` で永続化済み human message と紐づく（R2）。
- `RuntimeSessionState.pending_queue: Vec<QueuedTurnInput>`: pending queue の所有者。
- `SendMessageResponse`（`queued_turn: Option<QueuedAgentTurn>`, `pending_queue`, `pending_queue_count`, `human_message` 等）: 既存の応答型をそのまま使う。stall 中フォールバックは、既存の実行中 queue 分岐（`337-372`）と同じ応答形（`queued_turn = Some(..)`）になる。
- queue 分岐では、失敗し得る session shell / session 一覧 / title projection を human message の永続化と queue 投入より前に解決する。storage の message append は index 修復を反映した canonical な post-write `SessionMeta` を返し、受理後はその meta から対象 summary を置換して再 sort する非 fallible 処理だけで応答を組み立てる。これにより queue 投入済みなのに frontend が `false` を受け取って再送する境界の曖昧さと、stale index 修復後の永続 count と応答 count の乖離を作らない。

### frontend

`sendMessage` の戻り値を、送信成否を呼び出し側が判定できる形へ変更する。

- 推奨: `Promise<boolean>`（`true` = 送信成功。turn 開始・queue 追加のいずれも成功として `true`。失敗時 `false`）。
  - 型定義 `UseAgentChatResult.sendMessage`（`useAgentChat.ts:120-126`）を `Promise<void>` → `Promise<boolean>` に更新。
  - `BoundSessionChat.handleSend` / `ChatSessionView.handleComposerSend` は戻り値をそのまま伝播。
- `MessageInput.onSend` の型は `Promise<boolean>` とし、`true` の場合だけ成功として入力状態をクリアする。
- `queued_turn` の有無で「turn 開始」か「queue 追加」かを区別できるが、入力クリアの判定はどちらも成功扱いのため boolean で十分。

（例外を投げる方式は「リスクと代替案」参照。既存の非 await 呼び出し側へ影響しない boolean 方式を採る。）

## 処理フロー

### backend: `send_message` の分岐（修正後）

```
resolve session / backend_id
recover_queued_turn_if_idle_without_runtime

steer_target =
    if backend_supports_steering(backend_id):
        stalled_active_turn_target(session_id)   // Some のとき steer 可能
    else:
        None                                     // 非対応 backend は steer しない

if is_turn_busy(session_id):
    if let Some(target) = steer_target:          // steering 対応 backend のみ
        target.runtime.steer(TurnInput { .. })   // 失敗時は human message を保存せずエラー伝播（既存挙動維持）
        human_message = add_human_message_internal(..)
        return send_response(queued_turn = None, ..)
    else:                                         // ← stall 中 / 非対応 backend はここへ落ちる（新経路）
        (human_message, persisted_meta) = add_human_message_internal(..) // R2: 永続化後の canonical meta も取得
        queued = QueuedTurnInput::new(.., editor_context)
        queued.existing_human_message_id = Some(human_message.id)
        state.pending_queue.push_back(queued)
        return accepted_queue_response(
            pre_commit_projection,
            persisted_meta,
            queued_turn = Some(view),
            pending_queue,
        ) // R1

// turn 非実行時（Idle）は従来どおり新規 turn を開始
human_message = add_human_message_internal(..)
start_turn_for_session(..)
return send_response(queued_turn = None, agent_message = Some(..), ..)
```

要点:

- 早期エラー return（`293-297`）を削除する。stall 中でも `active-turn steering is not available` を返さない（R1・R6）。
- steer 分岐を `backend_supports_steering` でガードする。これにより stalled + 非対応 backend は既存の queue 分岐（`337-372`）を再利用して積まれる（full-recompute 経路を新設しない）。
- `stalled_active_turn_target` は steering 対応時のみ呼ぶ。非対応時は runtime 欠落による `No active agent runtime` エラー（`1642-1646`）にも到達しない。
- 将来 backend が実 steer を実装した場合、stall 観測中は `steer_target` が Some になり steer 経路が優先される（behavior「stall 判定中の実 steer 対応 backend では steer 経路が優先」を満たす）。通常の streaming turn は `stalled_active_turn_target` が None のため、既存どおり queue へ積む。

### backend: queue の実行（turn 終了後）

既存経路をそのまま使う。turn 終了時に `start_next_queued_turn`（`recover_queued_turn_if_idle_without_runtime` 経由、`703-711`）が pending queue から次の turn を起動する。テストでは `drain_next_queued_turn_for_test`（`1246`）で確認する。

### frontend: 送信と入力クリア

```
MessageInput.handleSubmit
  submitContent(content):
    mentions = syncMentionsForSubmit(..)
    const ok = await onSend(content, images, mentions)   // 完了を待つ（R3）
    if (ok === true) {                                   // 成功時のみクリア
        clearSubmittedSnapshot()                         // 待機中の追加入力は保持
    }
    // 失敗時は入力状態を保持（P5 / R3）
```

- `onSend`（= `handleComposerSend` → `handleSend` → `sendMessage`）の Promise を `await` し、`true` の場合だけ成功として扱う。
- 送信中は次の submit を受け付けず、同じ snapshot の重複送信を防ぐ。待機中に編集されたテキストや追加された添付は送信時 snapshot と区別し、成功時もクリアしない。
- `sendMessage` は成功で `true`、失敗で `false` を返し、失敗を握りつぶさない（R4）。
- backend が stall 中も成功応答（`queued_turn = Some`）を返すため、通常経路として `true` → クリアされ、queue チップに表示される（R5）。
- 送信中（未確定）はクリアしない（behavior「送信中に入力が即時破棄されない」）。

## エラー処理

- **backend**: stall 中 / steering 非対応の送信はエラーを返さず `Ok(SendMessageResponse)`（R1）。永続化失敗（`add_human_message_internal` の I/O エラー）や `start_turn` 失敗は従来どおり `AgentRuntimeError` として伝播する（stall フォールバックが新たなエラーを握りつぶすことはしない）。steer 対応 backend での steer 失敗は human message を保存せずエラーを返す既存挙動を維持する（`test_stall_signal後のsteer失敗はhuman_messageを保存しない` を壊さない）。
- **queue 受理境界**: session shell / session 一覧の読込失敗は queue commit 前に返す。human message 永続化・queue 投入後は取得済み projection から成功応答を構築するため、frontend の `false` は「queue に未投入」を意味し、同じ入力の再送で重複 queue を作らない。
- **frontend**: `sendMessage` は送信失敗時、入力保全のため `false` を返す。ユーザー向けバナー（`SET_ERROR`）は残すが、`active-turn steering is not available` を含む backend 生エラーは backend 側で発生しなくなるため UI に露出しない（R6）。バナー文言に backend 内部文字列をそのまま埋め込まない方針を維持する。
- 失敗時に入力を保持することで、ユーザーは失われていない入力から再送できる（behavior「送信失敗時に入力欄が保持される」）。

## テスト方針

### backend（`runtime/usecase.rs` の `#[cfg(test)]`）

- **更新／置換**: `test_stale_watchdog_無進捗turnをstall_signalに留めruntimeを閉じない`（`7561-7651`）を新仕様へ反転する。
  - 変更前: `send_message` が `active-turn steering is not available` で `expect_err`、`pending_queue` が空。
  - 変更後: `send_message` が `Ok` を返し、`queued_turn` が `Some`、`pending_queue` が非空、`add_human_message_internal` により human message が永続化され `existing_human_message_id` で紐づく。watchdog の非破壊挙動（`Reconnect` あり / `Interrupt`・`Close` なし、phase = Streaming、SessionState = Active、stall 通知発火）は維持を確認する。
- **新規**: stall 中送信 → queue 追加 → turn 終了後 drain。
  - stall 観測中に `send_message` → `pending_queue.len() == 1`、本文・画像・mention・editor_context が欠落なく保持されること（R2）。
  - `drain_next_queued_turn_for_test`（または turn 終了）で queue のメッセージが後続 turn として起動され、`pending_queue` が空になること（behavior「queue に積んだメッセージが turn 終了後に実行される」）。
- **回帰**: steering 対応 backend では steer 経路が優先されること（`test_stall_signal後のsend_messageはactive_turnへsteerしqueueしない`、`7653-` を維持）。
- **回帰**: stale index / orphan message chunk がある状態で queue を受理し、session 一覧 projection が後続読込不能になっても、commit 前に取得した projection と append が返す canonical meta から成功応答を返すこと。応答と永続状態の message count が一致し、human message と queue entry が 1 件だけ作られること。

### frontend

- `MessageInput.test.tsx`: `onSend` が `true` へ解決したときのみ送信対象の入力・添付・pasted text・mention がクリアされ、`onSend` が reject / `false` を返したときは入力状態が保持されることを検証（P5 / R3）。送信中（Promise pending）は破棄されず、待機中の追加入力・添付が成功後も保持され、二重 submit が直列化されること。
- `useAgentChat` のテスト: `sendAgentMessage` が失敗したとき `sendMessage` が `false` を返し（swallow しない）、成功時 `true` を返すこと（R4）。`vi.mock("@tauri-apps/api")` で invoke を stub。

### 品質チェック

`pnpm lint` / `pnpm test` / `pnpm build`、`cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`、`pnpm test:integration` を通過させる。

## リスクと代替案

- **意図的仕様の反転（issues-1301 D16/F-2）**: 「stalled retry/continue must not be silently queued」を反転する。requirements の Decisions で合意済み。ワークフロー node session の stall 中送信も queue へ積まれるようになるため、当該テストと関連する watchdog 挙動（非破壊 recovery）が維持されることをテストで固定する。
- **frontend 戻り値方式（boolean vs 例外）**: 例外送出（rethrow）だと `sendMessageRef` 経由の非 await 呼び出し側（Workflow panel 等）に unhandled rejection を波及させうる。影響範囲を局所化するため、後方互換な `Promise<boolean>` を採用する。既存の戻り値未使用の呼び出し側は影響を受けない。
- **queue と steer の同時対応 backend（将来）**: `steer_target` を `backend_supports_steering` でガードするため、実 steer 実装時は stall 観測中に steer 経路が優先される。stall 観測前の通常の streaming turn は既存の queue 経路を維持する。
- **queue 非永続化（OB-3）**: session close / backend 切替 / 再起動で queue は消える。本 ISSUE の対象外（別 ISSUE）であり、振る舞いとして保証しない。

## 仮定

- spec-id は `issues-1403`（ブランチ `feat/issues/1403` に整合）。正本は `specs/milestone-84-agent-chat-stabilization/`（OB-2 / I6 / P5）。
- 「queue へ積む」対象は steering 非対応 backend の実行中 turn（stall 判定中を含む）への送信。実 steer 実装時も、steer 経路を優先するのは stall 観測中の active turn に限る。
- queue 投入時の human message 永続化・queue 紐付けは、既存の実行中 queue 分岐（`runtime/usecase.rs:337-372`）と同一の仕組みを再利用し、新規スキーマを追加しない。
- frontend 改修は「クリアを成功応答後に移す／失敗を伝播させる」の最小範囲に限る。`MessageInput` の即時クリアと `useAgentChat` の catch swallow の 2 点が対象。
- R5（queue チップ）は既存の `pendingQueue` レンダリングで満たされ、backend が queue へ積めば追加実装は不要。
- `sendMessage` の戻り値は `Promise<boolean>`（成功=`true`）とし、turn 開始・queue 追加の双方を成功として扱う。

## Open Questions

なし。
