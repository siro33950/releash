# 実装レビュー（第1回）— feat-issues-1301

- 対象: worktree `feat/issues/1301` の未コミット実装（163 ファイル、+11,766 / −40,582。merge-base = main `3371adc2`、差分は `git diff HEAD`）
- 正典: `docs/specs/feat-issues-1301/{requirements.md, behavior.md, design.md}`
- レビュー方式: 8 観点並列（不具合根本原因 / 要求・振る舞い充足 / design 準拠 / 構造 / 品質 / テスト / セキュリティ / アーキテクチャ）+ 品質ゲート実行
- 指摘は全て実コード確認済み（file:line 付き）。scope:diff のみ FAIL 判定対象

## 総合判定: **FAIL**

**骨格は design どおり成立している**: 依存方向（domain 外部依存 0 / backend 実装は domain のみ依存 / adaptor→infrastructure::agent_session 全廃）、§12 削除一覧と grep 確認は全合格、backend_id 分岐排除、bridge 全廃、Entity/trait 定義、auto-allow 表、固定モデルカタログ、codex 0.139.0 のバージョン検証記録（§7.5）は準拠。

しかし **実行側（design §8）の中核が「型と語彙だけ実装され、駆動系が未配線」**の状態にある。報告された不具合（UI で streaming 状態が出ない）はその一症状で、同根の欠陥が turn 終端・permission 解決・復旧・event log・workflow 通知に体系的に存在する。加えて `#![allow(dead_code, unused_imports)]` が crate 全域に追加され、コンパイラと clippy による欠落検出が無効化されている（品質ゲートが green である理由の一つ）。

## 品質ゲート結果

| ゲート | 結果 | 注記 |
|---|---|---|
| `cargo fmt --check` | PASS | |
| `cargo clippy -- -D warnings` | PASS | **crate 全域 `#![allow(dead_code, unused_imports)]` により dead code 検出が無効化されている（G-1）** |
| `cargo test` | PASS（2,305 件） | **旧実装比 −507 件。旧 runtime_support/bridge 系 550+ 件が移植されずに削除（G-3）** |
| `pnpm lint` / `pnpm test` | PASS（1,304 件） | |

## モジュール別判定

| モジュール | 判定 | blocker/major/minor（scope:diff） |
|---|---|---|
| 0. 不具合根本原因調査 | FAIL | 3 / 6 / 0 |
| 1. 要求・振る舞い充足 | FAIL | 7 / 7 / 6 |
| 1b. design 準拠 | FAIL | 7 / 11 / 4 |
| 2. 構造 | FAIL | 2 / 5 / 5 |
| 3. 品質 | FAIL | 6 / 12 / 4 |
| 4. テスト | FAIL | 4 / 5 / 2 |
| 5. セキュリティ | FAIL | 1 / 5 / 3 |
| 6. アーキテクチャ原則 | FAIL | 1 / 3 / 2 |

以下は 8 モジュールの指摘を重複排除・統合した確定指摘一覧。ID は修正プロンプトから参照する。

---

## A. 報告された不具合の根本原因（UI で Think/返答中が出ず Stop/Interrupt が消える）

症状: UI からメッセージ送信すると turnPhase が idle のままになり、Thinking 表示と Stop/Interrupt ボタン（`MessageInput.tsx:799` の `isStreaming` 依存）が出ない。Workflow 起動時のみ表示される。

原因は 3 つの独立した欠陥の複合。Workflow 経路で表示されるのは、step panel（`BoundSessionChat.tsx:100`）が turn 開始**後**に `loadSession→get_session` で live phase をミラーすることと、workflow 専用の status 同期（`state_notification_gateway.rs:103-142`）が生きているためで、UI 送信経路にはどちらも無い。

### A-1（blocker）turn 開始時に `agent-session-state-changed(Streaming)` を emit しない
- 場所: `src-tauri/src/usecase/agent_session/runtime/usecase.rs:872-907`（`start_turn_for_session`）、`usecase.rs:1389-1499`（`start_next_queued_turn` の drain 成功パス）
- 旧実装（session_lifecycle.rs）は turn 開始後に Streaming を emit していた。frontend の `isStreaming` はこのイベントか `get_session` 再取得でしか更新されない。
- 修正: `runtime.start_turn` 成功後に `notifier.session_state_changed(&session_id, TurnPhase::Streaming, ..)` を emit する（両パス）。usecase に回帰テストを追加。

### A-2（blocker）`AgentStatusCenter` が未使用（`_status_center`）で status 系イベントが UI 経路で死んでいる
- 場所: `usecase.rs:160`
- 旧 `notify_status_transition`（HEAD `shared.rs:851`）相当が消え、`session-status-changed` 等 status 系 4 イベント（session list / status bar のスピナー供給元）が UI 起動 turn で一切 emit されない。design §9.2「status 系 4 種は意味不変」違反。
- 修正: phase 遷移点（turn 開始 Streaming / PermissionRequested WaitingPermission / complete_turn 終端 / Fatal）で status center 更新 + `AgentStatusNotifier` emit を移植する。

### A-3（blocker）`agent-turn-prepared` payload の camelCase 化で frontend listener と不一致
- 場所: `src-tauri/src/adaptor/presenter/agent_session.rs:24`
- `#[serde(rename_all="camelCase")]` が付き `chatSessionId/humanMessage/agentMessage` で emit されるが、listener（`useAgentSdkListeners.ts:44-49,150-167`）は snake_case を期待。全フィールド undefined で TypeError。
- 修正: rename を除去し旧形状（`chat_session_id`/`session`/`human_message`/`agent_message`）で emit。presenter に payload 形状テストを追加。

---

## B. turn 終端処理の欠陥（blocker 群）

### B-1（blocker）`complete_turn` の最終永続化が構造的に不達
- 場所: `usecase.rs:1342-1362`
- `streaming_message_id` を None 代入（:1343）した**後に**読んで message_id に詰める（:1356-1360）ため常に None。completed_at 付き最終 persist が全 turn でスキップされ、`streaming_parts` の解放（issues-1194 不変条件）も行われない。
- 修正: None 代入前に message_id を退避 → 最終 persist 実行 → `streaming_parts.clear()`。終端処理の単体テスト追加。

### B-2（blocker）成功 turn の session state が `Idle`（`Done` であるべき）
- 場所: `usecase.rs:1381`
- 既存規則（`lifecycle_controller.rs:33-41`: exit 0 → Done）と乖離。Done でのみ発火する webhook 完了通知（`notification_wiring.rs:93`）が消え、UI の done 表示も出ない（design §5.3 / requirements:53 違反）。
- 修正: Completed→Done / Failed・Timeout→Error / Interrupted(Abort)→Idle にマップ。

### B-3（blocker）workflow への turn 完了通知が未配線 → workflow agent step が進行不能
- 場所: `usecase.rs:1212-1327` 付近（`complete_turn`）
- `WorkflowRuntimeCommandUsecase::complete_turn`（`usecase/workflow/runtime_command.rs:77`）の production 呼び出し元が 0 件。workflow が turn 完了を検知できない（design §8.2 / requirements:43,44）。
- 修正: complete_turn 内で final parts / exit_code / failure_signal / token_usage から `WorkflowTurnCompleteNotification` を生成し workflow usecase へ通知する。

### B-4（major）`agent-session-state-changed` が常に exit_code=0 / interrupted=false / session_state=null
- 場所: `presenter/agent_session.rs:48-58`
- Fatal/Interrupted/Failed も正常完了として届き、frontend の `!interrupted` ガード（`useAgentSdkListeners.ts:348-366`）が無効化。
- 修正: notifier port のシグネチャを拡張し、TurnResult 由来の exit_code（0/1/124）/ interrupted / session_state を payload に載せる。ad-hoc `json!` を typed payload struct に変える。

### B-5（blocker）event log（保存正典）への記帳が全欠落
- 場所: `usecase.rs:1130` 付近（event 適用経路全体）
- `TurnStarted` / `FinalPartsRecorded` / `TurnCompleted` / `PermissionRequested` / `PermissionResolved` が一切 events.json に記帳されず、D1（Turn = event log 表現）・D15・issues-1247 の保存正典が空になる。projector の workflow_turn_complete 投影もデッドコード化。再起動後の状態復元が壊れる。
- 修正: design §8.1 手順 4 / §8.2 の記帳・projection を event_apply に実装する。

### B-6（major）turn latency telemetry（`releash.agent.turn.duration_ms`）の記録が消滅
- 場所: `other/telemetry/mod.rs:221,470`（histogram 定義のみ残存、production 呼び出し 0 件）
- 修正: event pump の turn 開始〜TurnCompleted で backend 非依存に記録（design §8.2）。

---

## C. permission の欠陥

### C-1（blocker）応答後も Permission part が永久 pending
- 場所: `usecase.rs:335-363`（`respond_permission`）
- design §8.2 の「live buffer の該当 part を Resolved に patch + force flush + `PermissionResolved` 記帳」が全て欠落。許可/拒否後も pending 表示のまま永続化され、reload 後に未解決カードが復活する（behavior「permission 履歴は変換後も理解できる」未充足）。
- 修正: 応答成功後に part を `Resolved{decision, answers}` へ patch → 即時永続化・flush → event log 記帳。WaitingPermission→Streaming 遷移の通知も A-2 と併せて実装。

### C-2（major）Claude の AskUserQuestion 回答が契約外の形で送信される
- 場所: `claude/permission.rs:171-180`
- answers を allow response の兄弟キーに置くが、SDK の PermissionResult(allow) に answers フィールドは無い。design §6.3 は `updatedInput` へ `{questions: 元, answers}` を合成と規定。
- 修正: design どおり updatedInput へ合成。Deny fallback（"User denied"）・Question 変換含めテスト追加。

### C-3（blocker）Codex の Deny 応答形式が design 違反 + テストの帳尻合わせ
- 場所: `codex/permission.rs:119`
- commandExecution / fileChange requestApproval の Deny を design §7.3 の `{"result":{"decision":"decline"}}` でなく全 method 一律 JSON-RPC error -32001 に変更し、**旧テスト（HEAD `codex_app_server.rs:2205-2237`）の decline 期待を error 期待に書き換えて固定**している（CLAUDE.md「テスト期待値を実装に合わせて変更しない」違反）。
- 修正: method 別に decline result / error 応答を分岐し、テストを旧意味（decline）に復元。

### C-4（major）Codex の user-input 要求（dynamicToolCall 経由）が応答不能
- 場所: `codex/convert.rs:219`、`codex/session.rs:160-166`
- `PermissionRequested` イベントにならず Pending part 直載せのため WaitingPermission 遷移せず、id=item_id は `respond_permission` の u64 parse で Invalid になり回答不能。さらに pending_methods に無い id を `unwrap_or_default()` で握りつぶし既定 accept を送る（セキュリティ指摘）。
- 修正: server request 経路（JSON-RPC id）に一本化し item 通知からの Permission part 生成を削除。未知 id は `AgentBackendError::Invalid` で拒否。

### C-5（major）`PermissionModeChanged` の resync 未実装
- 場所: `usecase.rs:1196-1198`（log 出力のみ）
- design §8.2（issues-947）: 保存値と比較し相違時に `runtime.set_permission_mode(saved)` を呼び戻す。
- 修正: resync を実装。`agent-permission-mode-changed` の emit も F-1 と併せて復旧。

### C-6（major）`pending_inputs` の無制限蓄積（full-retention / メモリ枯渇）
- 場所: `claude/session.rs:338-356`
- 全 can_use_tool の input（Write 全文含む）を保存し、削除は respond 時のみ。auto-allow される大半の要求分がセッション寿命でリークする。
- 修正: Prompt に上げた request のみ保存し、TurnCompleted で残存を破棄。

### C-7（minor）`respond_permission` の request_id 一致検証なし
- 場所: `usecase.rs:335`
- stale な request_id でも常に phase=Streaming / pending クリアに遷移する。
- 修正: 保持中 pending_permission_request の id と照合し、不一致はエラーまたは no-op。

---

## D. 復旧・プロセス管理の欠陥

### D-1（blocker）stale 監視（design §8.4）が全欠落
- 場所: `stale.rs` 不在。`last_progress_at` は記録のみ（`usecase.rs:1298`）、`SessionSpec.stale_timeout` 常に None（`usecase.rs:939-941`）
- workflow step の stale_timeout が無効化され、無進捗 turn（特に Codex ハング）が永久 Streaming で放置される。
- 修正: runtime_driver のタイマー駆動で超過検出 → `Interrupted{Timeout}`（exit 124 相当・中立文言の Error part）終端 → `interrupt()` → 10 秒 grace → `close()` → runtime 破棄。meta の workflow_step_context から SessionSpec へ timeout 類を配線。

### D-2（blocker）復帰計画（design §8.5）と Mismatch 処理が未実装
- 場所: `usecase.rs:1143-1153`（`SessionEstablished` の ResumeOutcome を読み捨て）、`usecase/runtime/context_restore.rs` 不在
- ContextRestorePlan（Resume/Reinject/NoContext）決定・Reinject prefix 前置・Mismatch 時の requeue→id クリア→再 open・ContextCarryState 永続化・`agent-session-context-carry-updated` 通知が全て無い。resume 失敗時に文脈が黙って失われる。
- 修正: design §8.5 を実装。

### D-3（blocker）Fatal（crash）後に pending queue が恒久スタック
- 場所: `usecase.rs:1410`（drain は live runtime 必須）、`usecase.rs:996-1004`（queue 非空で busy 扱い）
- crash 後の session は新規 send も queue に積むだけで二度と実行されない（design §8.2「queue は保全し、drain が再 open を含む」未実装）。
- 修正: drain 時に runtime 不在なら再 `open_session` する経路を追加し、Fatal ハンドラまたは次回 send から drain を起動。

### D-4（blocker）Fatal 時に `close()` を呼ばず子プロセスが孤児化
- 場所: `usecase.rs:1260`（runtime=None にするだけ）
- kill_on_drop 未設定のため子プロセス残存。parse Err 経路では stdout 読み手を失い永久ハング。
- 修正: Fatal branch で `runtime.close().await` を呼ぶ。

### D-5（blocker）`infrastructure/process/`（design D13/§2.1）が全欠落
- 場所: `claude/process.rs:70-198`、`codex/app_server.rs:26-49`、`lib.rs`（起動時 cleanup 配線削除）
- PID 登録（save_pgid）・orphan cleanup・CleanupGate・child env（CLAUDECODE/CLAUDE_CODE_ENTRYPOINT 除去、PATH alias・RELEASH_DATA_DIR・RELEASH_SESSION_ID・RELEASH_BASE_BRANCH 注入）が全て消滅。**アプリクラッシュ後、`--allow-dangerously-skip-permissions` 付き agent プロセス群が回収不能で残留する（セキュリティ blocker）**。`$RELEASH_SESSION_ID` 依存のプロンプト（`usecase/comment/mod.rs:264`）や releash CLI 連携（`cli/common.rs:50`）も壊れる。Claude Code 配下からの起動では CLI が nesting 検出で起動拒否。
- 修正: design どおり `infrastructure/process/{pid_registry,child_env}.rs` を実装し、両 backend の spawn・起動時 cleanup（lib.rs）に配線。setsid spawn / JSONL framing / 段階 shutdown の重複（claude/process.rs:99 と codex/app_server.rs:49）もここへ集約。

### D-6（blocker）`start_turn` 失敗時に phase が Streaming のまま固着 → session 永久 busy
- 場所: `usecase.rs:886`（queued 経路 :1489-1497 はロールバックしており不整合）
- 修正: Err 時に phase=Idle / streaming_message_id=None へ巻き戻す。

### D-7（major）Codex の startup retry / timeout・resume 判定・EOF 終端が未実装
- 場所: `codex/session.rs:273-291`、`codex/convert.rs:53`
- `wait_for_thread_id` 固定 15 秒・再 spawn なし・`SessionSpec.startup_timeout/max_retries` 未消費。resume は常に `NotRequested` 判定（要求 id 未追跡）で、tracked error 時の `BackendSessionCleared`+`Fatal` も無く、`state.thread_id` を spec.resume で先埋めするため死んだ thread に turn/start を投げて無期限ハング。turn 中 EOF で `TurnCompleted(Crash)` を先行 emit しない（契約 4 違反）。
- 修正: `retry_startup_until_ready` 相当を実装、requested_resume_id を追跡して Resumed/Mismatch 判定、tracked error 分岐、EOF 時の Crash 先行 emit。

### D-8（blocker）Claude の非 JSON stdout 1 行で turn が Crash 終端
- 場所: `claude/process.rs:144`（parse 失敗を Err で返す）、`claude/session.rs:330`
- design §6.1 の前方互換規約（speculative parse・上限 1MB・非 JSON 行無視）違反。CLI の警告 1 行で落ちる。CLI バージョンチェック（>= 2.0.0）も未実装。
- 修正: parse 失敗行は蓄積・再試行 or skip で継続。Err は I/O エラーのみ。バージョンチェック追加。

### D-9（blocker）Claude の interrupt 相関・resume rollback が未実装
- 場所: `claude/convert.rs:176`、`claude/session.rs:192`
- interrupt 後の result が Completed/Failed として記帳される（design §6.2 の wasAborted 優先・10 秒 Abort 合成が無い）。ユーザーの Stop 操作が Error 表示になる。interrupt 後の resume rollback（最後に成功した turn の session_id へ戻す）も欠落。
- 修正: aborting 状態を保持し result 到着時 `Interrupted{Abort}` 優先、10 秒タイマー合成、resume rollback 実装。旧 `bridge-utils.test.mjs` のテスト意味を Rust テストへ移植。

### D-10（major）stderr を piped のまま誰も読まない → pipe 詰まりでハング
- 場所: `claude/process.rs:98`、`codex/app_server.rs:45`
- 修正: stderr reader task（log::warn へ流す）を常駐させる。

### D-11（major）Codex shutdown が即 SIGKILL（pgid sweep なし）
- 場所: `codex/app_server.rs:26-28`
- trait 契約（graceful teardown）違反。setsid した process group の孫プロセスが残存し得る。
- 修正: stdin EOF → SIGTERM(-pgid) → 待機 → SIGKILL(-pgid) の段階 shutdown に統一（D-5 の共有 utility 化と併せて）。

---

## E. streaming / parts 管理の欠陥

### E-1（blocker）merge_part の三重実装（実行経路は縮退版）→ part 無限増殖
- 場所: `usecase.rs:1560`（縮退版: Text/Thinking/Permission のみ統合）、`domain/agent_session/entities/message_part.rs:63`（正典・production 未使用）、`projector.rs:591-`（第三実装）
- ToolResult 累積・ToolUse in-place・TaskStatus 更新・Todo 単一スロット等が実行経路に無く、**Codex の outputDelta が delta 毎に別 ToolResult part として無限 append** され UI と保存が崩壊する。design D1「単一実装に集約」違反。
- 修正: live buffer を domain Entity で保持し `entities::merge_part` を適用、縮退版を削除。projector も委譲（不可なら逸脱記録）。merge_part 全規則のテスト補完。

### E-2（blocker）Claude の assistant text/thinking 二重変換 → 本文の二重表示・二重保存
- 場所: `claude/convert.rs:246`
- `--include-partial-messages` の stream delta 蓄積と assistant message の text/thinking block 変換が重複。design §6.2 の表に assistant text 行は無い（旧実装も tool_use のみ処理）。
- 修正: assistant からは tool_use（と TodoWrite→TodoListSnapshot）のみ変換。

### E-3（major）coalescing / 1 秒間隔永続化 / resync snapshot / rollback retry の全欠落
- 場所: `usecase.rs:1270-1325`（`apply_parts`）
- delta 毎に全 parts を clone → 即 emit + `persist_message_parts` で全量ディスク書き（1 turn で O(n²) I/O）。design §8.3（issues-1214）と §8.2（1 秒間隔 persist）違反。旧 `stream_emit.rs` の 93 テストも未移植（`streaming.rs` 自体が不在）。
- 修正: flush 判定純関数を `usecase/agent_session/runtime/streaming.rs` として移植し、タイマー駆動 coalescing（33ms/1000parts/256KiB）+ 1 秒間隔 persist + 終端 flush に戻す。テスト移植。

---

## F. イベント・surface の欠落

### F-1（major）design §9.2 で「不変」の 4 イベントが未 emit
- 場所: emit 箇所 grep 0 件（frontend は listener 保持のままデッド化）
- `agent-pending-message-consumed` / `agent-models-updated` / `agent-session-context-carry-updated` / `agent-permission-mode-changed`。queue 表示減算・queued human message の transcript 反映・model 同期・context carry 表示が途絶。
- 修正: presenter/notifier port に 4 イベントの emit を復旧。

### F-2（major）Codex の thread lifecycle / skills / fuzzy search が no-op 化
- 場所: `codex/models.rs:68` 付近（archive/unarchive no-op、fork Ok(None)）、skills/list・fuzzyFileSearch ワンショット未実装
- design §7.4 / D9 指定の既存機能（thread アーカイブ同期・fork・native 検索・runtime skill catalog）が消失（requirements:53 違反）。
- 修正: 旧 `thread_lifecycle_gateway.rs` / `skill_catalog_gateway.rs` / `codex_fuzzy_file_search_gateway.rs` のワンショット JSON-RPC 実装を CodexBackend メソッドへ移植。

### F-3（major）`GetSessionResponse.can_change_backend` 未実装で frontend 判定が残存
- 場所: `BoundSessionChat.tsx:207`（backend 変更可否判定）、同 :196-202（default model フォールバック。§9.4-5 で削除指定）
- 修正: Rust 側で can_change_backend を供給し frontend 判定・フォールバックを撤去。

### F-4（major）`PermissionDialog.tsx` に Rust presentation ロジックの TS 再実装
- 場所: `PermissionDialog.tsx:89`（`presentationFromRequest`）
- `present_agent_permission_request_inner` の kind 判定・編集可否・plan/questions 抽出を TS で 1:1 再実装（rust-first-logic.md / DRY 違反）。
- 修正: presentation を Rust から供給（PermissionRequestMsg または pending_permission_request payload に同梱）し TS 実装を削除。

### F-5（minor）slash commands の description/argument_hint 消失
- 場所: `claude/convert.rs:348`（system/init の名前配列のみ使用）
- design §6.1 は initialize control_response の `commands`（説明付き）から発行と規定。
- 修正: initialize 応答を処理して SlashCommand を構築。

### F-6（minor）compact_boundary / 現行 listener 表示の system subtype 対応が未実装
- 場所: `claude/convert.rs`（対応表・テスト固定なし）
- 修正: design §6.2 の system subtype 行を実装し convert.rs テストに固定。

---

## G. 品質・規約・テスト

### G-1（blocker）crate 全域 `#![allow(dead_code, unused_imports)]` + デッドコード 20 件超
- 場所: `lib.rs:3`（+ `domain/agent_session/gateway.rs:1`、entities/value_objects/claude/codex/runtime 各 mod.rs にも一括 allow が散在）
- CI の clippy -D warnings による欠落検出を crate 全体で無効化。配下に参照ゼロの新規デッドコード群: steer trait メソッド、`SessionSpec.startup_timeout/max_retries`（write-only）、`entities/{message,session}.rs` 全体、`TurnId`/`TurnStopReason`、`QueuedTurn`、`is_turn_terminal`、codex wire の METHOD_* 6 定数、`todo_items_from_value`、`wire_mode_for_spec`、両 backend の `cli_path()`、write-only フィールド（latest_usage/thread_id/generation/last_progress_at）、`app_server.rs:96 shutdown`、`_status_center`、`AgentTurnMetric` 等。
- 修正: 一括 allow を全て除去。各警告は「配線して使う（本レビューの欠落実装と対応）/ 削除 / Issue 参照付き item 単位 allow」のいずれかで解消。

### G-2（major）D10（backend_id 必須化）未実施
- 場所: `usecase/agent_session/session/mod.rs:977`（`Option<String>` のまま）、既存テスト `chat_session_without_backend_id_deserializes` が欠損許容を固定
- 修正: meta 読込時に backend_id 欠損を invalid session 隔離へ落とし、必須化。storage round-trip テストで固定。

### G-3（blocker）テストの大量削除と §15 必須テストの欠落
- Rust テスト 2,812 → 2,305（−507）。旧 runtime_support/bridge 系 550+ 件の大半が「意味を保った移植」なしに削除。
- 欠落: usecase 実行側の必須シナリオ（turn 手順 / queue の Fatal 後保全 / permission 遷移順序 / post-turn 更新 / stale / Mismatch→Reinject / persist-first）はテスト皆無（trivial 2 件のみ）。claude convert は §6.2 の 5/13 行のみ、codex convert も一部のみ。旧 `session_lifecycle.rs` 62 件・`recovery.rs` 59 件・`stream_emit.rs` 93 件・`context_restore.rs` 10 件・`bridge-utils.test.mjs` の移植先なし。
- 修正: `test_support` の TestAgentRuntime をイベント注入可能にし、§15 の必須シナリオを網羅。変換表は行単位の wire フィクスチャテストで固定。

### G-4（major）構造の重複・肥大
- `usecase.rs` が 1,840 行の god-file（イベント適用・queue drain・prompt 合成・DTO 変換・lock が同居。`event_apply.rs` は名前に反し DTO 変換のみ）
- `build_queued_system_prompt` と `build_turn_system_prompt` の重複（`usecase.rs:1526/1053`）
- setsid spawn・JSONL framing の両 backend 重複（D-5 で集約）
- `scan_dir` の claude/codex 完全重複（`codex/skills.rs:22` / `claude/skills.rs:22`）、thread start/resume builder の準重複（`codex/session.rs:309/333`）
- `RuntimeSessionPhase` が `TurnPhase` の 1:1 重複（`session_state.rs:12`）
- 修正: design §2.1 の分割（event_apply/streaming/stale/context_restore/queue/system_prompt）に沿って再配置し重複を統合。

### G-5（major）`tempfile` が unix 限定依存で Windows ビルド破壊
- 場所: `Cargo.toml:71`（`[target.'cfg(unix)'.dependencies]`）に対し `claude/process.rs:37` は cfg ガードなしで使用
- 修正: 通常の `[dependencies]` へ移動。

### G-6（minor）registry の個別 `app.manage` / `try_state` service-locator の温存
- 場所: `lib.rs:106`、`adaptor/gateway/workflow/{runtime_engine_impl.rs:752, runtime_session.rs:112, state_notification_gateway.rs:154}`、`application_lifecycle.rs:9`
- design §11「全廃」に対する未達。段階対応なら design に逸脱を追記。

### G-7（minor）残骸: 空モジュール `adaptor/controller/agent_session/mod.rs`（message_dispatch.rs 削除後）、`event_log/mod.rs:11` の stale doc（runtime_support 参照）
- 修正: 削除・修正。

### G-8（minor）テスト命名規約違反
- 新規テストが `test_{業務機能(日本語)}_{条件と期待結果}` 規約（TEST.md）に従っていない（usecase.rs の 2 件、message_part.rs、convert 系）。GWT 構造も未明示。
- 修正: 規約準拠に改名・構造化。

### G-9（minor）Claude permission の updated_input parse 失敗を `.ok()` で握りつぶし元 input で allow 送信
- 場所: `claude/permission.rs:160`
- 修正: parse 失敗はエラー伝播。fallback は updated_input=None の場合に限定。

---

## 既存注意事項（scope:touched、FAIL 判定外）

| ファイル:行 | 内容 |
|---|---|
| `adaptor/controller/command/agent_session/session.rs:5`（stored_session.rs:6 / suggestion.rs:3 も同様） | controller が `infrastructure::platform::app_data_dir::resolve_data_dir` を直接 import（既存不整合の温存）。data_dir 解決の usecase 注入への統一は別 Issue |

## 良好な点（維持すること）

- 依存方向: domain 外部依存 0 / usecase→外側 0 / backend 実装は domain のみ依存 / adaptor→infrastructure::agent_session 全廃
- design §12 削除一覧・grep 確認（CODEX_BACKEND_ID == / unwrap_or(CLAUDE_BACKEND_ID / agent-sdk-message / "type":"interrupt" / "type":"setModel" / codex_file_change）全合格
- §6.3 auto-allow 表（wire mode 基準・plan 行含む）準拠、固定モデルカタログ・表示名の移設、§7.5 バージョン検証（codex-cli 0.139.0）の記録
- spawn 引数 / stdin の JSON エスケープ、session_id の UUID 検証、instruction/edit preview のパストラバーサル防御、updated_input の境界検証、ログへの payload 非出力

## 修正の優先順位

1. **P0（ユーザー可視の機能停止）**: A-1〜A-3、B-1〜B-5、C-1、D-3〜D-6、E-1、E-2
2. **P1（design 必須機能の欠落・堅牢性）**: C-2〜C-5、D-1、D-2、D-7〜D-11、E-3、F-1、F-2、B-6
3. **P2（規約・品質・テスト）**: G-1〜G-9、C-6、C-7、F-3〜F-6

修正完了の判定は §15 のテスト要件と design §16 M7 の受け入れ確認（品質ゲート + 手動確認手順）に従う。**G-1（一括 allow 除去）と G-3（テスト復元）は他の修正の検出器なので、P0 と並行して最初に着手すること。**
