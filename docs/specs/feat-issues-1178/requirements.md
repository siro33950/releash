# Requirements

## Type

不具合修正 / 堅牢性改善（resilience）

## Goal

Claude Agent SDK 経路で Opus 4.8（および他モデル）との通信が無音停止しても、Releash 上の agent turn が永続的に Thinking 表示のまま固まらないようにする。具体的には、SDK からの `result` 到着だけを turn 完了条件として信用せず、Rust runtime 側で「進捗のない turn（stale turn）」を検出し、必ず turn を失敗状態として完了させ、次のメッセージ送信を可能にする。あわせて、停止した Node bridge を破棄・再生成し、壊れた SDK client 状態を次 turn に持ち越さない。

完了時には、SDK から `result` が返らない再現ケースでも、UI が永続 Thinking にならず、stale timeout 後に該当 turn が失敗として完了し、部分出力が残り、ユーザーが再試行できる状態になる。

## Background

Releash は Claude CLI を TTY で直接操作するのではなく、Rust runtime から Node bridge（`src-tauri/resources/claude-sdk-bridge.mjs`）を起動し、`@anthropic-ai/claude-agent-sdk` の `query()` を介して Claude Code と通信している。

現状の turn ライフサイクルはイベント駆動であり、turn の完了は SDK/bridge からの `result`（`message.type === "result"`）を起点として bridge が `turn_complete` を出力し、Rust 側（`bridge_common.rs`）の `run_turn_complete_transition_locked()` が `TurnPhase::Streaming` → `TurnPhase::Idle` へ遷移、フロント（`useAgentSdkListeners.ts` → `agentChatReducer.ts`）が `SET_TURN_PHASE` で UI を更新する、という流れで成立している。

このため、SDK iterator が `result` を返さず無音でハングすると、以下の連鎖で永続 Thinking が発生する。

- bridge: `for await (const message of currentQuery)` がブロックし、`gotResult` が false のまま `turn_complete` を出さない。
- Rust: turn は `Streaming` のまま保持され、状態遷移が起きず `streaming_parts` が蓄積され続ける。
- フロント: `turn_phase === "streaming"` が維持され、`ThinkingPart` が `isStreaming=true` で `animate-pulse` を継続表示する。

現状の Rust runtime には turn 完了を待つ watchdog や absolute/idle timeout が存在しない（タイムアウトはセッションクローズ時の `CLOSE_TIMEOUT_SECS = 5` のみ）。Node bridge のプロセス再生成（restart）も session 単位の新規 spawn のみで、turn 単位の復旧手段はない。

### native（Claude Code CLI）側の既存防御と盲点

bridge が起動する `claude` CLI（調査時点 v2.1.112）の実装（`cli.js`）を確認した結果、無音 stall に対する native の防御は存在するが、本不具合の故障モードには構造的な盲点があることが判明した。

- **API リトライ（既定10回）**: `maxRetries` の既定は `10`（`CLAUDE_CODE_MAX_RETRIES` で上書き可）。ただし発火条件は HTTP エラー応答（408/409/429/≥500/`x-should-retry`）か transport 例外のみ（`shouldRetry`）。無音 stall は 200 OK のまま応答も例外も出ないため、リトライ枠は一度も使われない。1リクエスト上限は `API_TIMEOUT_MS`（既定 600000ms = 10分）。
- **byte 単位 idle watchdog（既定オン）**: SSE body を監視し、最低300秒（`Math.max(CLAUDE_STREAM_IDLE_TIMEOUT_MS||90000, 300000)`）バイトが途絶えると `StreamIdleTimeoutError` でストリームをエラーにし、部分出力をフラッシュして clean に抜ける。`CLAUDE_ENABLE_BYTE_WATCHDOG` を falsy にしない限り有効。ただしタイマーは**あらゆるバイトでリセット**されるため、Anthropic API の SSE `ping` キープアライブが届く限り発火しない。
- **event 単位 watchdog（既定オフ）**: `CLAUDE_ENABLE_STREAM_WATCHDOG` 未設定時は無効（`f1()` 内 `if(!s6)return`）。有効時は実ストリームイベント毎にタイマーをリセットし、`CLAUDE_STREAM_IDLE_TIMEOUT_MS`（既定 90000ms、**clamp なし**）の半分で warning、満了で「`Streaming idle timeout … aborting stream`」を出してストリームを abort、部分出力をフラッシュして clean な idle-timeout error を throw する（`cli_streaming_idle_timeout` / `cli_stream_loop_exited_after_watchdog`）。重要なのは、SDK の SSE デコーダが `if(w.event==="ping")continue` で **ping をイベントとして yield しない**ため、このタイマーは **ping ではリセットされない**点。したがって event 単位 watchdog は「ping 生存・content 停止」を**正しく検出できる**——ただし既定オフのため、現状の Releash 起動では働いていない。

結論として、native は「**接続は byte 的に生きている（ping は届く）が `result` も意味あるデルタも来ない**」状態を**既定構成では**捕捉できない（byte watchdog は ping でリセット、event watchdog は既定オフ）。これが永続 Thinking の直接原因。対策の方向は2つあり、両立する: (1) bridge env で `CLAUDE_ENABLE_STREAM_WATCHDOG=1` を有効化し native 側に一次防御を効かせる、(2) Releash 側で進捗イベント（thinking/text/tool/progress デルタ）ベースの stale 検出を持つ。(2) は byte ではなく**意味ある進捗**で測るため ping に騙されず、かつ native ではなく bridge/SDK 自体がハングした場合も拾える最終手段となる。

公開 issue（claude-agent-sdk-typescript #333 / #339 / #348、claude-code #54434 / #63583）および他ツールの回避策（OpenClaw / tg_content_factory / PocketPaw / MassGen / clawcodex）から、SDK / stream-json / SSE 系で終端イベント欠落または無出力 stall が起きるケースが確認されており、consumer 側で watchdog・defensive fallback・client 破棄を入れる方針が共通している。Releash でも同様に、外部 SDK/CLI が無音停止しても UI 状態を回復できるようにする必要がある。

## Users / Actors

- Releash デスクトップアプリで Claude（Opus 4.8 等）と対話する開発者
- workflow/headless 経路から agent を利用するユーザー
- Releash 内で turn を進行する AgentChat / agent session runtime（Rust）
- `query()` を介して Claude Code と通信する Node bridge

## Scope

- Rust の agent session runtime において、turn ごとに「最後に bridge/SDK から進捗イベントを受信した時刻」を記録する。
- `Streaming` / `WaitingPermission` の turn が一定時間進捗なしの場合に stale turn として検出する。
  - Claude 4 系の thinking / tool / progress イベントを liveness（進捗）として扱う。
  - text delta だけでなく thinking delta も liveness として扱う。
- stale 検出時に、対象 turn へ合成エラーイベントを流し、必ず `turn_complete` 相当の状態遷移（`TurnPhase::Idle`、失敗扱い）を発生させる。
- stale 検出時に Node bridge へ abort/interrupt を送り、一定時間で正常に戻らなければ bridge プロセス（プロセスグループ）を kill/restart する。
- stale で完了させる際、それまでの部分出力（`streaming_parts`）を失わず persist し、UI に「Claude 応答が停止したため中断した。再試行できる」旨の状態を表示する。
- bridge プロセスへ、native の idle 防御を Releash の turn timeout と整合させる環境変数を設定する。設定対象は実在する lever（`CLAUDE_STREAM_IDLE_TIMEOUT_MS` / `CLAUDE_ENABLE_STREAM_WATCHDOG` / `CLAUDE_ENABLE_BYTE_WATCHDOG` / `CLAUDE_CODE_MAX_RETRIES` / `API_TIMEOUT_MS`）とする。**注意**: 当初想定していた `CLAUDE_CODE_STREAM_CLOSE_TIMEOUT` は CLI v2.1.112 に存在せず、無効である。
- `result` なしで stream/bridge loop が閉じた場合に、正常終了扱いにせず bridge/client を破棄して次 turn に持ち越さないようにする。
- permission 待ち（`WaitingPermission`）にも別 timeout を設け、headless 経路で永久待ちにならないようにする。
- Stop（ユーザー中断）操作と stale timeout 操作が競合しても、二重完了/二重エラーにならないよう排他制御する。
- 上記を unit test または統合テストで検証する。

## Non-goals

- Anthropic 側（Opus 4.8）のモデル障害そのものの解消。Releash は外部が無音停止しても UI 状態を回復できるようにするのみ。
- SDK/CLI 通信プロトコルや `@anthropic-ai/claude-agent-sdk` 自体の改修・差し替え。
- Claude 以外の agent backend（Codex / Cursor 内蔵 AI 等）への同等機構の新規導入（本対応は Claude SDK 経路を対象とする。共通化が自然な場合の流用は妨げないが、目標には含めない）。
- リトライ（自動再送）や自動 resume による回復。本対応は「失敗として完了させ、ユーザーが手動で再試行できる状態にする」までを範囲とする。
- turn timeout 値をユーザー設定 UI として公開すること（**仮定**: 本対応では固定の定数値とする。設定可能化は別途）。
- AgentChat の権限モデルや承認 UI のデザイン刷新。
- 永続 Thinking 以外の UI 表示課題（Thinking 蓄積量の soft limit / drain 等）の対応。

## Requirements

### 検出（stale detection）

- agent session runtime は、turn ごとに最後の進捗イベント受信時刻を記録すること。
- `Streaming` 状態の turn が、進捗イベントなしで stale timeout（既定 **180 秒**）を超過した場合に stale と判定すること。
- `WaitingPermission` 状態の turn は、**headless 経路に限り** permission 用 timeout（既定 **300 秒**）を超過した場合に timeout と判定すること。デスクトップ（人が応答しうる経路）では permission 応答を無期限に待ち、timeout しないこと。
- Claude 4 系の thinking delta / tool イベント / progress イベントの受信を進捗として扱い、これらが届く間は stale 判定しないこと。
- **仮定**: stderr のみの api/request ログは liveness として扱わない（無音 stall を進捗ありと誤判定しないため）。

### 復旧（recovery）

- stale 検出時、対象 turn を失敗状態として完了させ、`TurnPhase` を `Idle` に遷移させること。これにより UI が永続 Thinking から復帰すること。
- stale 完了後、同一セッションで次のメッセージを送信できる状態になること。
- stale 完了時、それまでの部分出力を失わず保持・表示すること。
- stale 検出時、Node bridge へ abort/interrupt を送ること。一定時間で turn が解消しない場合は bridge プロセスを kill し、必要に応じて再生成すること。
- stale で bridge を停止/再生成した後、壊れた SDK client 状態が次 turn に混入しないこと。
- `result` を受信しないまま stream/bridge loop が閉じた場合も、正常終了扱いにせず bridge/client を破棄し、次 turn に持ち越さないこと。
- stale 完了時、UI に「Claude 応答が停止したため中断した。再試行できる」旨の状態を表示すること。

### 競合制御

- Stop（ユーザー中断）操作と stale timeout 操作が同時に発生しても、turn の二重完了/二重エラーが起きないこと（既存の per-session runtime lock を用いた排他で保証する）。

### 設定

- native の idle 防御を Releash の turn timeout と整合させるため、実在する環境変数（`CLAUDE_STREAM_IDLE_TIMEOUT_MS` / `CLAUDE_ENABLE_STREAM_WATCHDOG` / `CLAUDE_ENABLE_BYTE_WATCHDOG` / `CLAUDE_CODE_MAX_RETRIES` / `API_TIMEOUT_MS`）を bridge プロセスの環境へ設定すること。`CLAUDE_CODE_STREAM_CLOSE_TIMEOUT` は存在しない env のため使用しないこと。

### テスト

- unit test または統合テストで以下を確認すること:
  - `result` 欠落（無音 stall）時に stale turn が失敗完了扱いになる。
  - bridge から進捗イベントが届き続ける間は stale timeout しない。
  - stale timeout 後に bridge プロセスが（必要に応じて）再生成される。

## Constraints

- 検出・復旧ロジックは Rust（agent session runtime）側に実装すること（プロジェクト方針: 全ロジックは Rust）。フロントは状態表示に徹する。
- 正当に長時間かかる Opus の thinking を stale と誤判定して中断しないよう、進捗イベント（thinking 含む）による liveness 判定を確実に行うこと。
- 既存の turn 完了経路（`result` → `turn_complete` → `run_turn_complete_transition_locked`）の正常系挙動を変えないこと。stale 経路は同じ終端状態（`TurnPhase::Idle`、失敗 exit_code）へ収束させること。
- 二重完了防止のため、turn 完了状態の遷移は per-session の排他ロック下で原子的に行うこと。
- bridge プロセスの kill はプロセスグループ単位で行い、孤児プロセスを残さないこと（既存の PGID/`killpg` 機構を踏襲）。

## Acceptance Criteria（概要）

- SDK から `result` が返らない再現ケースでも、UI が永続 Thinking にならない。
- stale timeout 後、該当 turn が失敗状態として完了し、次のメッセージ送信が可能になる。
- timeout 時に部分出力が残る。
- timeout 時に Node bridge が停止/再生成され、壊れた SDK client 状態が次 turn に混入しない。
- Stop 操作と timeout 操作が競合しても二重完了/二重エラーにならない。
- 上記をカバーする unit/統合テストが存在し、`result` 欠落時の stale 完了・進捗継続時の非 timeout・timeout 後の bridge 再生成を確認できる。

## Assumptions（仮定）

- 本対応の対象は Claude Agent SDK 経路（`claude-sdk-bridge.mjs` + `bridge_common.rs`）に限定する。
- 失敗完了後の回復はユーザーの手動再試行で行う（自動リトライ/自動 resume は含めない）。
- turn timeout / permission timeout は固定の定数値とし、ユーザー設定 UI は設けない。
- stale turn timeout（Streaming 進捗なし）の既定値は 180 秒とする（保守的設定。正当に長い Opus thinking の誤中断を避ける）。
- permission 応答待ち timeout は headless 経路に限り適用し、既定値は 300 秒とする。デスクトップ経路では permission を無期限に待つ。
- stale 判定の進捗イベントには thinking / tool / progress / text delta を含め、stderr のみの api/request ログは含めない。
- Spec ディレクトリ ID は worktree/ブランチ名に合わせ `docs/specs/feat-issues-1178` とする。

## Open Questions

なし
