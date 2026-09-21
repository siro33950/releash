# Context

- 正本: [#1839 \[workflow\] Workflow の操作を Abort だけにし、再開は Session の Resume と Command の Retry に揃える](https://github.com/siro33950/releash/issues/1839)
- 所属: [マイルストーン #90「01. Workflow 操作と状態の簡素化」](https://github.com/siro33950/releash/milestone/90) の着手順 2。着手順 1 の [#1838](https://github.com/siro33950/releash/issues/1838) は close 済み（`9b30f626 refactor(workflow): 未使用の中断・再開実装を削除 (#1855)`）
- [#1826](https://github.com/siro33950/releash/issues/1826) は「Abort の整理に依存するため」本 ISSUE の後に着手する。Archive を 1 つの操作にし、内側で Abort を使う変更は #1826 が持つ。Abort が worktree のフォルダの存在を確かめない変更は、#1826 が言う「Abort の整理」として本 ISSUE が行う
- [#1836](https://github.com/siro33950/releash/issues/1836)（着手順 3）が、起動時に「プロセスを失った」を記録する処理の置き換えを行う
- [#1678](https://github.com/siro33950/releash/issues/1678) は、起動に失敗した Session Node が Resume も Retry も受け付けず実行木が復旧不能になる不具合であり、本 ISSUE の対応で原因が無くなる
- 永続化は event store（事実ログ）であり、Node の状態は事実からの導出である。Node の `Paused` は事実として記録されない（`src-tauri/src/adaptor/gateway/workflow/fact_log.rs:262-263`）
- Command の非ゼロ exit は Node の失敗ではない。`docs/glossary/WORKFLOW.md:234` が「process 起動不能は Node failure、非ゼロ exit code または stdout 検証失敗は `ok: false` の確定結果になる」と定め、`:423` が「Command の `ok` は宣言なしで boolean routing field として使える」と定める。実装も一致している
- `docs/glossary/DOMAIN.md` の「状態所有」「隔離 worktree」、`docs/glossary/WORKFLOW.md` の境界表・`on_failure`・delegate、`docs/guide/workflow/concepts.md` の「利用者の操作」、`docs/guide/workflow/yaml.md` / `common.md` / `lua.md` の `on_failure` と Diagnostic の記述が、本 ISSUE で記述を変える正本とガイドである
- AgentSession の `open` / `paused` / `archived` は AgentSession が所有する別の集約であり、本 ISSUE では変えない

# Outcome

対象者は、Releash で workflow を実行し、止まった Node を再開したりやり直したりする利用者と、Releash のコードを読み変更する開発者である。

現在、「プロセスが居ないので起動し直したい」という同じ状況に対して、Workflow の Stop / Resume、Node の Retry、Session の Resume という重なった操作があり、Node の状態も `Paused` / `Failed` / `Running` の 3 通りに分かれている。その結果、どの操作を押せばよいか判断できない、Workflow の Resume は Node 1 つの失敗で実行木全体が通らない、一度も起動できなかった Session はどの操作も受け付けない、プロセスを失った Command は Retry できない、worktree のフォルダが消えた実行は Abort できない、という状態になっている。

また `Failed` は、Session でも Command でも「基盤が動かせなかった」ことしか表していない。Session は Submit でも Stop でも Contract 違反でも失敗にならず、Command の非ゼロ exit は `ok: false` の確定結果として完了する。それにもかかわらず workflow 定義には、失敗したときの扱いを宣言する `on_failure` がある。

変更後は、実行木に対して実行の状態を変える操作は Abort だけになり、止まった作業を動かし直す操作は Session の Resume と Command の Retry の 2 つになる。Node の状態は中断も失敗も持たず、終わっていない Node は実行中のままで、「プロセスが居るか」が状態とは別の 1 項目として読める。基盤が動かせなかった場合は規定回数まで自動で起動し直し、それでも動かなければ利用者の操作を待つ。結果の良し悪しは Command の `ok` と定義の辺だけが表す。

# Current Behavior

調査時点は 2026-09-21、branch `feat/issues/1839`、`7948c6ec fix(review): backend再接続後の差分更新停止を修正 (#1856)`。

## 実行木に対する操作

- 画面の Workflow メニューは Stop / Resume / Abort / Archive を並べる（`src/components/workspace/WorkspaceList.tsx:385-402`）。呼び出しは `stop_workflow` / `resume_workflow` / `abort_workflow`（`src/lib/workflowExecutionActions.ts`）
- Tauri / Connect の入口は `src-tauri/src/adaptor/controller/client/workflow/shared.rs:669-692`（resume）・`:784-808`（stop）と `runtime.rs`、proto は `rpc StopWorkflow` / `rpc ResumeWorkflow`（`proto/client.proto:3486`・`:3498`）と `StopWorkflowRequest` / `ResumeWorkflowRequest`（`:2497`・`:2590`）、oneof の項目（`:132`・`:144`）
- local API は `/v1/workflow/executions/{execution_id}/stop` と `/v1/workflow/executions/{execution_id}/resume`（`src-tauri/src/adaptor/controller/api/workflow.rs:79-86`）
- usecase の port は `WorkflowStopExecutionGateway` / `WorkflowResumeExecutionGateway`（`src-tauri/src/usecase/workflow/ports.rs:150-158`）で、実装は `stop_execution.rs` / `resume_execution.rs`（配置は `src-tauri/src/usecase/workflow/command/` 配下）と `runtime_command_gateway.rs`
- gateway の実体は `stop_workflow_execution`（`src-tauri/src/adaptor/gateway/workflow/workflow_host/lifecycle_commands.rs:175`）と `resume_workflow_execution`（`:478`）

## Workflow の Stop / Resume が行うこと

- Stop は事実を記録しない。メモリ上で Node を `Paused` にし、Session のターミナルへ Ctrl-C を送り、Command のプロセスを止める。`WorkflowEvent::NodePaused` は事実ログへ書かれない（`fact_log.rs:262-263`）。再起動すると Stop したことは残らない
- Resume（Session）は Session 自身の再開と同じ `ensure_provider_running`（`src-tauri/src/usecase/agent_session/agent_session_lifecycle.rs:173-194`）を呼び、そのあと `"Continue the paused workflow node from the existing conversation context."` を 1 行送る（`lifecycle_commands.rs:683`）
- Resume（Command）は再開ではなく、Retry と同じ仕組みで新しい attempt として起動し直す（`restart_paused_command_node`、`NodeRestartMode::CommandResume`、`can_restart_paused_command`）
- Resume は、親 Session に未注入の child の結果があればそれを渡す（`lifecycle_commands.rs:661-749`）。この経路は Workflow の Resume にしかない
- Resume は実行単位で行われ、途中の Node で失敗すると `restore_unactivated_resumes_after_failure` で元へ戻し、実行木全体が再開しない
- 新しい attempt を作るのは `restart_node_attempt_at`（`src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs:2080-2121`）だけで、手動 Retry と `on_failure: retry` が通る。Resume 経由の Session 再開は attempt を増やさない

## Abort と Retry の worktree の扱い

- Abort / Stop / Resume は入口で `validate_execution_command_target`（`lifecycle_commands.rs:795-836`）を通る。ここが `worktree_resolver.resolve` を呼び、`normalize_worktree_filter_path`（`src-tauri/src/adaptor/gateway/workflow/worktree_gateway.rs:21-28`）が `canonicalize` するため、フォルダが存在しないと `invalid worktree_path` で拒否される
- Retry はこの検証を通らない。UI は `retry_workspace_node`（`src/hooks/useWorkspaceNodeDetail.ts:182`）、local API は `/v1/workflow/executions/{execution_id}/retry` から `retry_node`（`src-tauri/src/usecase/workflow/control_plane.rs:367-397`）へ入り、worktree のフォルダの有無を先に確かめる処理は無い
- 隔離 worktree の branch と path は attempt を埋め込む（`src-tauri/src/domain/workflow/value_objects/worktree_origin.rs:117-137`）。フォルダを消しても branch は残るため、同じ attempt で作り直すと `repo.branch(..., false)` が衝突する（`worktree_gateway.rs:147-172`）

## Node の状態と「プロセスが居ない」の表れ方

`NodeExecutionStatus`（`src-tauri/src/domain/workflow/value_objects/node_execution.rs:100-108`）は `Unresolved` / `Running` / `Paused` / `WaitingApproval` / `Succeeded` / `Failed` / `Aborted` を持つ。

| 場面 | Node の状態 | 導出元 |
| --- | --- | --- |
| Session のプロセスが正常終了した | `Paused` | `fact_replay.rs:443-444` `derive_session_process_exit` |
| Session のプロセスが異常終了した | `Failed`（origin は `ProviderProcessExit`） | `fact_replay.rs:429-442` |
| Session / Command の起動や runtime が失敗した | `Failed`（origin は `Runtime`） | `fact_replay.rs:451-454` `derive_leaf_failed` |
| Command が 0 以外の終了コードで終わった | `Succeeded`（`ok: false` の Artifact を出して完了） | `workflow_host.rs:208-270`・`:1731-1735`・`:1798-1880` |
| Command のプロセスを失った（終了コードが無い） | `Paused` | `fact_replay.rs:416-420` |
| 記録が残らないまま落ちた | `Running` のまま | — |

- 「プロセスが居るか」を状態と独立に読む項目は、Session 側にだけある（`ManagedPtyPresence`、`src-tauri/src/domain/agent_session/aggregates/agent_session.rs:248-252`）。値は `Live` / `ConfirmedAbsent` / `Unknown` の 3 つで、`Unknown` は terminal の所有者が一致しない場合である（`provider_agent_terminal_gateway.rs:44-59`）。Command 側には無い。実行中の command はメモリ上の `active_commands`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:117`）に載るが、read model へは出ない
- ツリーの状態分類（`classify_own_status`、`src-tauri/src/domain/workspace_tree/value_objects/mod.rs:170-202`）は、`Paused` を `Idle`（緑）に、`Failed` を `Failure`（赤）に分類する。`Running` の Session Node は AgentSession の活動状態で `Active`（青）/ `Attention`（黄）に分かれ、`Running` の Command Node は `Active` になる。色は `src/components/workspace/WorkflowNodeStatusIcon.tsx:20-25`
- 生の失敗理由は read model へ出ない。`error_reason` には "Workflow node failed" という定型文だけが入る（`src-tauri/src/domain/workspace_tree/projection.rs:769-806` のテストが保証している）

## Session と Command の失敗の意味

- Session Node が `Failed` になる経路は、プロセスの異常終了（`fact_replay.rs:431-442`）と runtime の失敗（`:451-454`）の 2 つだけである
- Session は Submit しても Stop しても失敗しない。`SubmitRejected`（Contract に合わない Artifact）は `Ok(())` で状態を変えず（`fact_replay.rs:466`）、`StopReceived` は `derive_session_settlement`（`mod.rs:3360-3378`）へ入り、この関数は「まだ待つ」「承認待ち」「完了」しか返さない。timeout による失敗の導出は無い
- Command Node が `Failed` になるのは、runner が結果を返せなかったとき（`fail_current_command_node`、`workflow_host.rs:1912`）と runtime の失敗である。runner が結果を返した場合は exit code を見ずに `commit_command_output` へ入り、`ok = exit_code == 0 && validation_success` を載せた Artifact を出して完了する

## on_failure の現状

- `on_failure` は children エントリが所有し、省略時は中断、`ignore` は失敗を除外して続行、`retry: n` は最大 n 回の自動再実行である（`docs/glossary/WORKFLOW.md:168-171`）
- `apply_on_failure_treatment` は `status == Failed` の Node にだけ効く（`mod.rs:3286-3288`）。予算は attempt から数える（`:3255-3268`）
- Diagnostic は `WFC009`（`ignore` の Artifact への依存）と `WFC010`（Sequence / Fanout child への `retry`）、および `WFS008`（`on_failure` の形の誤り）である
- `workflows/` の builtin 定義にも、`~/Library/Application Support/releash/workflows/` の Lua 定義にも、`on_failure` の宣言は 1 つも無い

## Retry と Resume の可否

- `can_retry`（`node_execution.rs:168-179`、`workflow_execution/mod.rs:230-239`）は `Failed`、または `Running` / `Paused` かつ完了信号が片方だけ（`completion_signals.is_partial()`）のときに true。Session と Command の両方に出る
- `can_restart_paused_command`（`workflow_execution/mod.rs:241-247`）は Command かつ `Paused` のときに true
- `resume_previous_state`（`:252-270`）は `Paused`、または Session かつ `Failed` かつ origin が `ProviderProcessExit` のときに値を返し、`can_resume` の元になる
- 読み側では `resume_eligible`（`src-tauri/src/domain/workspace_tree/projection.rs:193`、`value_objects/mod.rs:136`）へ写され、Workflow Node の `can_resume` / `can_stop` は葉の状態から再計算される（`src-tauri/src/domain/workspace_tree/entities/mod.rs:638-688`）。DTO・proto・生成 TypeScript・手書き TypeScript（`src/types/workspace-tree.ts:43-48`）の各層に同じ項目がある
- 操作の提示は read model の capability が決め、frontend はそれを描くだけである（`src/components/panels/NodeContentView/NodeContentView.tsx:210-214`、`src-tauri/src/usecase/workflow/workspace_tree.rs:94-97`）。受付側の `retry_node_once`（`control_plane.rs:372-397`）も同じ domain の述語を通る
- 画面の Node 側は Retry ボタンだけを持ち、Session の再開は AgentSession のパネルの Resume（`src/components/panels/AgentSessionPanel/AgentSessionPanel.tsx:230-238`）で、AgentSession の lifecycle が `paused` のときにだけ出る

## #1678 の状況

起動直後にプロセスが消え、`session_attached` が無いまま `process_exited` した Session Node は `session_id` を持たない。Workflow の Resume は `session_id` を必須にしているため実行木全体が通らず、`can_retry` も「一度は動いた証跡」を要求するため false になる。どちらの操作も受け付けない。

## 正本とガイドの記述

- `docs/glossary/DOMAIN.md:112`: NodeExecution が所有する状態として `Paused` と `Failed` を挙げている
- `docs/glossary/DOMAIN.md:126`: 隔離 worktree の「実体が失われた場合の再開は process の起動失敗として扱い、Retry は新しい attempt の worktree を作る」
- `docs/glossary/WORKFLOW.md:11`: runtime の所有として「Node の実行、Artifact、辺の進行、stop / resume / abort」
- `docs/glossary/WORKFLOW.md:168-171`: `on_failure` の意味と `WFC009` / `WFC010`
- `docs/glossary/WORKFLOW.md:197`: 「`on_failure: ignore` の失敗 slot はキーの欠番になる」
- `docs/glossary/WORKFLOW.md:325`: delegate の「注入前の中断では resume 時に未注入の結果を返す」「親の provider session を復元できない場合は既存の失敗経路と手動 Retry に委ねる」
- `docs/guide/workflow/concepts.md:189-197`: 利用者の操作の表に Stop / Resume / Abort / Retry / Approve を並べている
- `docs/guide/workflow/yaml.md:172-192`・`:266-272`・`:288`・`:362-372`: `on_failure` の例と構文の表
- `docs/guide/workflow/lua.md:360`: `on_failure = r.retry(n)` の制約
- `docs/guide/workflow/common.md:49`・`:136`・`:182-183`: 予約語の一覧と `WFS008` / `WFC009` / `WFC010`

# Scope / Non-goals

## 変更する対象

- 実行木に対する Stop と Resume の操作。画面のメニュー、Tauri / Connect の入口、proto の RPC と message と oneof の項目、local API の 2 ルート、usecase の port、gateway の実装
- Workflow の Resume が持っていた、未注入の child の結果を親 Session へ渡す処理の実行契機
- 再開した Session への `"Continue the paused workflow node from the existing conversation context."` の自動送信
- `Paused` の Command を起動し直す専用経路（`restart_paused_command_node`、`NodeRestartMode::CommandResume`、`can_restart_paused_command`）
- Session Node の手動 Retry と、Retry の可否のうち「Submit と provider Stop の片方だけ届いている」条件
- Node の状態 `Paused` と `Failed`、およびそれらを作る・戻す遷移と、`failure` / `failure_kind` / `origin` の保持
- 起動や runtime が失敗した Node の自動的な起動し直し
- workflow 定義の `on_failure`（`retry: n` / `ignore` / 省略時の中断）の構文と、その評価・Diagnostic（`WFC009`、`WFC010`、`WFS008` の該当部分）
- Command の「プロセスが居るか」の判定の追加と、Session / Command 双方の在否の読み取り
- プロセスが居ない Node のツリー上の状態分類
- 読み取り側の `can_stop` / `can_resume` / `resume_eligible` とその再計算、および DTO・proto・生成 TypeScript・手書き TypeScript の各層
- Abort が対象の worktree のフォルダの存在を確かめる処理
- 上記に対応するテスト
- `docs/glossary/DOMAIN.md`、`docs/glossary/WORKFLOW.md`、`docs/guide/workflow/concepts.md`、`docs/guide/workflow/yaml.md`、`docs/guide/workflow/common.md`、`docs/guide/workflow/lua.md` の該当記述

## 変更しない対象

- AgentSession の `open` / `paused` / `archived` の lifecycle と、その遷移・記録
- 新しい attempt を作る仕組み（`restart_node_attempt_at` の採番と記録形式）
- Command が 0 以外の終了コードで終わったときに `ok: false` の Artifact を出して完了する振る舞いと、`ok` を routing field として使う設計
- Archive を 1 つの操作にし、内側で Abort を使う変更、および消えた worktree の片付け。#1826 が扱う
- 起動時に「プロセスを失った」を記録する処理の置き換え。#1836 が扱う
- Approve と Submit の操作
- Retry が worktree のフォルダを必要とすること。Retry はそのフォルダでプロセスを起動し直すため、フォルダを必要とする性質を変えない
- 事実ログの記録先と形式

# Requirements

- R-001: 実行木に対して利用者が行える、実行の状態を変える操作は Abort だけである。実行木を止める操作と実行木を再開する操作は、画面・CLI・local API・Connect のどの入口にも無い
- R-002: Abort は、対象の実行木の worktree のフォルダが存在しなくても実行できる
- R-003: 終わっていない Session Node は、プロセスが居ないときに Resume できる。再開できる会話も実行できる作業場所も無い場合（一度も起動できなかった、隔離 worktree の実体が失われた）でも Resume でき、その場合の Resume は新しい attempt での起動として振る舞う
- R-004: Session Node を利用者が手動で Retry する操作は無い
- R-005: Resume した Session に対して、Releash が会話の続きを促す指示を自動で送ることはない。新しく起動する場合にその Node の最初の指示を送る振る舞いは、通常の起動と同じである
- R-006: 親 Session へまだ渡していない child の結果は、その Session Node の Resume のときに親 Session へ渡る
- R-007: Command Node は、プロセスが居ないときに Retry できる
- R-008: Node の状態に、中断して再開を待つことを表す値と、失敗を表す値は無い。終わっていない Node の状態は実行中である
- R-009: 終わっていない Session Node と Command Node について、プロセスが居るかどうかが、状態とは別の 1 項目として外部から読める。この項目は表示と操作の可否の判定に使う
- R-010: 欠番（R-008 へ統合）
- R-011: 完了信号が Submit と provider Stop の片方だけ届いていることを理由に Retry できる Node は無い
- R-012: 取り除いた proto の RPC、message、oneof の項目、field の番号と名前は、以後別の意味で再利用されない
- R-013: workflow 定義に、child が失敗したときの扱い（自動のやり直し、失敗の無視）を宣言する構文は無い
- R-014: `docs/glossary/DOMAIN.md`、`docs/glossary/WORKFLOW.md`、`docs/guide/workflow/concepts.md`、`docs/guide/workflow/yaml.md`、`docs/guide/workflow/common.md`、`docs/guide/workflow/lua.md` の、利用者の操作・Node の状態・workflow 定義の構文の記述が、変更後に実際に行える操作、読み取れる状態、書ける定義と一致している
- R-015: Session Node と Command Node の起動が失敗した場合、規定回数まで自動で起動し直す。起動し直しは 1 回ごとに新しい attempt を作り、試行の間隔は回を追うごとに長くなる。規定回数を使い切っても起動できなかった Node は、実行中かつプロセスが居ない状態で利用者の操作を待つ
- R-016: プロセスが居ない Node は、実行木の状態分類として、利用者の介入を待つものとして読める
- R-017: Command Node が 0 以外の終了コードで終わった場合、その Node は完了し、結果は `ok` を含む Artifact として後続の判断に使える

# Assumptions / Open Questions

なし。
