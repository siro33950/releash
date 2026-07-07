# Requirements

## Goal

Agent session の turn 完了・中断判定は、既存の backend 由来の明示的な turn 終端イベントを正系とする。

無出力 timeout が turn のエラー終端、timeout interrupt、runtime close を合成する経路をなくす。

無反応 session への timeout 処理は、turn 完了・中断判定とは別に扱う。無出力 timeout は、backend の明示的な終端イベントが欠落している可能性、transport / stream / backend process の異常、または長時間無出力だが処理が継続している状態を観測するための補助 signal として扱う。timeout 到達だけで turn を完了・失敗・中断扱いにせず、session / runtime を継続可能に保つ。

timeout 到達時に自動実行できる処理は、session / runtime を破棄しない範囲の recovery に限る。例: stream / transport の再接続、backend-owned state の再読込、明示的な retry / continue / abort を選べる利用者・workflow 介入点の提示。

user 明示 cancel、workflow で設定された wall-clock / run timeout、backend からの明示 terminal / fatal event、tool 固有 timeout は、無出力 timeout とは別経路として扱う。

## Background

現状の Releash には、backend 由来の明示的な turn 終端イベントを検知する経路が存在する。Codex backend の `turn/completed` と Claude backend の `result` は `AgentRuntimeEvent::TurnCompleted` へ変換され、runtime usecase 側で turn completion として処理される。

一方で stale watchdog は、agent session が `Streaming` のまま一定時間出力しない場合に、`TurnResult::Interrupted { reason: Timeout }` を合成し、workflow notification を出し、runtime に interrupt を送り、grace period 後に runtime close まで進める。このため、無出力状態の観測、turn の中断完了、runtime の破棄が同じ経路に混在している。

OSS 調査では、無出力や stream 異常を turn 完了の代替 signal として扱う実装は一般的ではなかった。調査対象は Codex、Claude Agent SDK / Claude Code issue、Cline、Continue、Roo-Code、Aider、Goose、opencode、Gemini CLI、Qwen Code、OpenHands。

Codex は `turn/completed` を明示的な lifecycle event として扱い、interrupt も `turn/completed status: interrupted` を待つ。Claude Agent SDK は final message を明示的な終了として扱い、process / connection failure は result message ではなく exception として扱う。Claude Code の issue でも、stream が途中で切れて terminal marker が欠落した場合は retry または warning すべき bug として扱われている。

Cline、Continue、Roo-Code は、abort / cancel / timeout を明示的な runtime 操作または tool / terminal 単位の timeout として扱う。stream failure は retry、error 表示、または user cancel と区別され、無出力だけで agent session 全体を正常完了または破棄しない。

Aider と Goose は、provider / API / network timeout を retryable error として分類し、backoff retry または利用者向け error として扱う。shell / terminal command timeout は tool 固有の結果や process cancel であり、agent session の turn 終端とは分離されている。

opencode は transport liveness に heartbeat を使い、active execution へ join / wake / interrupt できる session coordinator を持つ。provider stream error は assistant / tool failure として記録される。Gemini CLI と Qwen Code は finish reason / terminal marker の欠落を invalid stream として扱い、retry、recovery、または error として処理する。Qwen Code の workflow stall watchdog は、tool 実行中は timer を停止し、stall した attempt を abort / retry するが、parent cancellation や user cancellation とは区別する。

OpenHands は `running`、`paused`、`finished`、`error`、`stuck` などの execution status を分け、cleanup / idle reap と active execution の終端を分離している。

このため、Releash の要件では、無反応 session を検知した後の処理を「turn completion の合成」ではなく、backend-owned state を保ったまま recovery / user intervention / workflow intervention へつなぐ監視・制御経路として定義する。

現状コードの具体挙動（`src-tauri/src/usecase/agent_session/runtime/`）:

- `stale.rs`: 基準 timeout は `DEFAULT_STALE_TIMEOUT`（180秒）、`workflow_step_context.stale_timeout_secs` で上書き可、上限 `MAX_STALE_TIMEOUT`（1800秒）。ToolResult 未着の ToolUse が `domain_streaming_parts` に残る場合のみ `effective_stale_timeout` で 1800秒まで延長。`turn_is_stale` は phase == `Streaming` かつ同一 generation かつ `now - last_progress_at >= timeout` で `true`。
- `usecase.rs` の `spawn_stale_watchdog_task`: stale 判定成立で `complete_turn(TurnResult::Interrupted { reason: Timeout, error: STALE_TIMEOUT_MESSAGE })` を呼び、workflow notification を dispatch し、`runtime.interrupt()` → `STALE_CLOSE_GRACE` 待機 → `runtime.close()` まで進める。
- `last_progress_at` の更新契機は、streaming の domain part 受信、`KeepAlive` 受信（phase != Idle）、permission 応答での streaming 再開の 3 つに限られる。この区間外は無出力扱いとなり、reasoning 中・ToolUse part 未到着・KeepAlive を送らない backend で誤検知が起きる。

## Users / Actors

- **agent session 利用者**: agent session を実行し、応答待ち・中断・継続を観測・操作する開発者。誤検知による中断で作業が止まることを避けたい。
- **workflow runtime**: agent session の turn 完了・中断を判断材料として次の step 制御や human checkpoint 提示を行う実行主体。turn 終端の正誤に依存する。
- **agent backend（Codex / Claude 等）**: 明示的な turn 終端イベント（Releash の現行経路では Codex `turn/completed` / Claude `result` 等）を発行する源。無出力 timeout は backend の代替判定であってはならない。

## Requirements

1. **完了・中断判定は backend の明示的終端イベントを正系とする。** turn の完了・失敗・中断は、backend 由来の terminal event（Releash の現行経路では Codex `turn/completed`、Claude `result`）を受信して初めて確定する。無出力時間の経過を turn 完了・中断の判定根拠にしない。

2. **無出力 timeout は補助 signal に限定する。** 無出力 timeout の到達は「backend 終端イベントの欠落可能性」「transport / stream / backend process 異常」「長時間無出力だが処理継続中」を観測する signal として扱う。到達しただけで turn を完了・失敗・中断扱いにしない。

3. **timeout 到達時に session / runtime を破棄しない。** 現状の `interrupt()` → `close()` 経路（無出力起因）を廃止し、runtime を継続可能な状態に保つ。backend-owned state を保持する。

4. **timeout 到達時の自動処理は非破壊 recovery に限る。** 許容するのは stream / transport 再接続、backend-owned state 再読込、および明示的な retry / continue / abort を選べる利用者・workflow 介入点の提示。session / runtime を破棄する自動処理は行わない。

5. **別経路の終端は従来どおり尊重する。** user 明示 cancel、workflow の wall-clock / run timeout、backend の明示 terminal / fatal event、tool 固有 timeout は、無出力 timeout とは独立した経路として引き続き turn / session を終端できる。

6. **暴走防止の上限を持つ。** 自動継続や再接続を行う場合、無限ループ・無制限リトライを防ぐ上限（回数・時間など）を設ける。上限到達後は利用者・workflow 介入点へ委ねる。

## 受け入れ基準の概要

- streaming 中に無出力 timeout（180秒 / 設定値）へ到達しても、`STALE_TIMEOUT_MESSAGE` によるエラー中断や runtime close が発生しない。
- reasoning 中・ToolUse part 未到着・KeepAlive 途絶といった「生きているが無出力」の区間で turn がエラー終端されない。
- backend の明示的終端イベント（Releash の現行経路では `turn/completed` / `result`）を受信したときに、turn が正しく完了・中断として確定する。
- 無出力 timeout 到達時、session / runtime が継続可能な状態のまま残り、利用者または workflow が retry / continue / abort を選べる、あるいは非破壊 recovery が試みられる。
- user 明示 cancel、workflow timeout、backend fatal event、tool timeout による終端は従来どおり機能する。
- 自動継続・再接続を行う場合、上限を超えたときに暴走せず介入点へ委ねる。

## Constraints

- 全ロジックは Rust（Tauri backend）に実装する。frontend は表示・入力・invoke に徹する（`rust-first-logic`）。
- backend-owned state を source of truth とし、full-retention / full-recompute 経路を増やさない。
- 対象範囲は主に `src-tauri/src/usecase/agent_session/runtime/`（`stale.rs`、`usecase.rs` の `spawn_stale_watchdog_task` 周辺）。
- 既存の workflow 設定項目（`stale_timeout_secs`、`startup_*`）との後方互換を壊さない。既存の設定意味の変更が必要な場合は Open Question / 後続 Spec で確定する。
- 直近修正（687cb7c9: stale watchdog 誤爆等の信頼性修正）と整合させ、退行させない。

## Scope

- 無出力 timeout（stale watchdog）が turn のエラー終端・interrupt・runtime close を合成する経路の廃止。
- turn 完了・中断判定を backend の明示的終端イベントへ寄せる方針の要件定義。
- 無出力 timeout を非破壊な観測・recovery / 介入点提示の signal として再定義する要件定義。
- 上記に伴う暴走防止上限の要件定義。

## Non-goals

- 復帰の具体実装方式（自動継続送信 / 非エラーの継続待ち UI / close せず保留の詳細、および上限の具体値）の確定。これは `behavior.md` / `design.md` で扱う。
- backend 側（Codex / Claude）の終端イベント仕様そのものの変更。
- user cancel、workflow wall-clock / run timeout、tool 固有 timeout の挙動変更。
- agent session 以外の workflow 経路の timeout 設計変更。
- frontend UI の詳細デザイン確定。

## 仮定

- 現状コードで確認した挙動（`spawn_stale_watchdog_task` が Timeout interrupt → close を行う、`DEFAULT_STALE_TIMEOUT` 180秒 / 上限 1800秒、`last_progress_at` の 3 更新契機）を現行仕様として扱う。
- backend の明示的終端イベントは `AgentRuntimeEvent::TurnCompleted` として既に runtime usecase へ届いており、これを完了判定の正系として利用できる。
- `workflow_step_context.stale_timeout_secs` は今後「補助 signal の発火閾値」として意味づけを保つ（廃止しない）。この意味変更方針は後続 Spec で確認する。

## Open Questions

- なし（復帰方式の詳細は Non-goals として扱い、`behavior.md` / `design.md` で確定する）。
