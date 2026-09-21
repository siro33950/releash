# Context

- 正本: [#1838 \[workflow\] 使われていない Workflow の中断・再開まわりの実装を削除する](https://github.com/siro33950/releash/issues/1838)
- 所属: [マイルストーン #90「01. Workflow 操作と状態の簡素化」](https://github.com/siro33950/releash/milestone/90) の着手順 1。後続の #1839 / #1836 / #1826 / #1840 / #1845 はこの ISSUE の完了を前提にしない独立した変更だが、同じ領域を触る
- `docs/glossary/DOMAIN.md`「状態所有」: WorkflowExecution は木全体の `Running` / `Completed` / `Aborted` を所有する。`WaitingApproval`、`Paused`、`Failed`、`Interrupted` は NodeExecution が所有する
- [#1835](https://github.com/siro33950/releash/issues/1835) は 2026-09-20 に close 済み。その対応 (`6cc1572a refactor(lifecycle): 終了処理を時間制限付きの後始末へ簡素化 (#1847)`) により、終了処理は「workflow が起動した command を止める」「terminal の状態を保存する」を時間制限付きで行う形になった
- 永続化は event store（事実ログ）であり、Workflow 実行の状態は事実ログからの projection として導出する

# Outcome

対象者は、Releash のコードを読み変更する開発者と、CLI / Connect API から Workflow 実行の状態を読む利用者である。

現在、「Workflow を中断して、あとで再開する」ためのモデルが、domain の型・遷移から usecase DTO、wire protocol、CLI 出力、CLI ガイドまで全層に残っている。実体は本番ビルドに存在せず、値を入れる処理もテスト専用である。このためコードを読むと Workflow が中断・再開できるように見え、API の型と CLI ガイドには決して出ない値が載り、変更のたびに使われない経路とそのテストを保守することになる。

変更後は、Workflow 実行の状態が `Running` / `Completed` / `Aborted` の3つだけになり、中断理由・再開位置の項目と、それらを保持・保存・復元する仕組みがどの層にも無い状態になる。本番で実際に出ていた値と、Workflow の起動・実行・完了・Abort の振る舞いは変わらない。

# Current Behavior

調査時点は 2026-09-21、branch `feat/issues/1838`、`4267f22f release: v0.4.15 (#1849)`。

## Workflow 実行の状態と遷移

- `ExecutionStatus`（`src-tauri/src/domain/workflow/value_objects/execution.rs:5-14`）と `RuntimeExecutionState`（`state.rs:14-22`）は `WaitingApproval` と `Interrupted` を `#[cfg(test)]` で持つ。本番ビルドには存在しない
- `stop()` / `interrupt()` / `resume()` / `request_approval()` / `approve()` / `reject()`（`src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs:4234-4327`）はいずれも `#[cfg(test)]`。本番に残る遷移は `abort()` だけで、その `ExecutionStateSet::Resumable` 分岐も `#[cfg(test)]`
- `ExecutionStatus::is_resumable()` は本番ビルドで常に `false` を返す。`can_stop()` / `can_resume()` / `can_abort()` はこれに依存し、呼び出し元は削除対象のメソッドだけである
- 一方、利用者の Stop / Resume 操作自体は本番に存在する。`stop_workflow` / `resume_workflow` は `controller/client/workflow/runtime.rs:57`・`:72` から `workflow_host` の `stop_workflow_execution` / `resume_workflow_execution` へ渡り、Node の状態を変える。Workflow 実行の状態は `Running` のまま変わらない

## 中断理由と再開位置

- `ExecutionInterruptionReason`（crash / stale / stop / orphan、`execution.rs:69-97`）と、`WorkflowExecution` の `interruption_reason` / `resume_from_node`（`execution.rs:165-166`）が存在する
- 値を入れる唯一の処理は `ExecutionStore::interrupt_execution_with_usage`（`src-tauri/src/adaptor/gateway/workflow/execution_store.rs:1063-1121`）で、テストからしか呼ばれない。事実ログからの導出 `fact_replay.rs:234-235` は常に `None` を入れる
- 同じ項目が `src-tauri/src/usecase/workflow/dto.rs:203`・`:250`、`src-tauri/src/adaptor/protocol/workflow.rs:60-67`・`:211-212`、`src-tauri/src/adaptor/presenter/workflow.rs:21-24`・`:75-92`、`src-tauri/src/adaptor/protocol/client/conversions.rs`、`proto/client.proto:1678-1679`・`:2993-2994`、`src/generated/client_types.ts`、`src/types/workflow.ts:208-209`・`:228-229` に残っている
- `WorkflowExecutionView` は両項目に `skip_serializing_if` を持たないため、`releash workflow status --json` と Connect API の応答は常に `"interruptionReason": null` と `"resumeFromNode": null` を出力する。一方 `WorkflowExecutionSummaryDto` は `skip_serializing_if` を持ち、出力には現れない

## 到達しない状態値

- `ExecutionStatusView`（`protocol/workflow.rs:41-47`）、`ExecutionStatusDto`（`dto.rs:198-204`）、proto の `ExecutionStatusView.Value` / `ExecutionStatusDto.Value` / `WorkspaceHistoryStatus.Value` は `waiting_approval` と `interrupted` を持つ。いずれも本番の経路では生成されない
- `src-tauri/src/cli/workflow.rs:88-95` の `execution_status_name` は5値を網羅する
- `docs/guide/cli.md:91`・`:104` は `releash workflow status` の状態値として `waiting_approval` と `interrupted` を載せ、`:110-111` は `interruptionReason` と `resumeFromNode` を載せている

## ファイル永続化

- `ExecutionStore::metadata_store()`（`execution_store.rs:666-674`）は `#[cfg(not(test))]` で常に `Ok(None)` を返す。`ExecutionMetadataStore` の `persist` / `remove` は本番ビルドで必ず `Err` を返し、`list_valid` と `data_dir` は `#[cfg(test)]`（`:552-598`）。`workflow_executions/*.json` の走査・原子的書き込みを含む一式がテスト fixture 専用である

## 本番の呼び出し元が無いメソッド

ISSUE が挙げるのは `sync_active_projection`、`complete_execution`、`interrupt_execution` 系、`reserve_interrupted_for_abort` / `commit_interrupted_abort` / `rollback_interrupted_abort`、`list_for_worktree`、`list_completed`、`list_executions`、`get_execution`、`list_non_terminal_metadata`、`resolve_execution_by_worktree`、`resolve_worktree_by_execution`、`active_len` である。調査で確認した点は次のとおり。

- `sync_active_projection` と `complete_execution` は引数違いの wrapper で、本番が使うのは `sync_active_projection_with_usage`（`workflow_host/runtime_commit.rs:88`・`:100`）と `complete_execution_with_usage` の側である
- `ExecutionStore::get_execution` / `resolve_worktree_by_execution` / `list_executions` は、`WorkflowReadUsecase` の同名メソッドとは別物である。本番経路（CLI の `file_direct.rs`、`controller/client/workflow/execution.rs`、local API の `api/workflow.rs`）が呼ぶのは `WorkflowReadUsecase` 側で、`ExecutionStore` 側ではない

## その他

- `recovery_effect_suppression`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:127`）は `:454` で生成され `:1215` で参照されるが、値を書き込む処理が無く常に空である
- `src-tauri/src/adaptor/controller/client/workflow/execution.rs:68` のコメントは `workflow_execution_logs/{execution_id}.ndjson` を engine の log source として説明するが、これを読み書きするコードは存在しない

## shutdown_all_active_commands / command_shutdown_intents

ISSUE 本文は「本番の呼び出し元が無い」としているが、調査時点ではいずれも本番から到達する。

- `workflow_host.rs:2014` の `shutdown_all_active_commands` は、`adaptor/gateway/application_lifecycle.rs:68`（`DaemonShutdownGateway::stop_commands`）→ `usecase/workflow/runtime_command.rs:116` → `adaptor/gateway/workflow/runtime_command_gateway.rs:307` の経路で、アプリケーション終了処理から呼ばれる
- `command_shutdown_intents`（`workflow_host.rs:124`）はその経路で書き込みと削除が行われ、`workflow_host.rs:1776-1785` で読まれる。ただし読んだ値は `log::debug!` を出すかどうかにしか影響しない

# Scope / Non-goals

## 変更する対象

- Workflow 実行の状態 `Interrupted` と `WaitingApproval`、および `Interrupted` を前提とした再開可能性の判定
- Workflow 実行を中断・再開・承認待ちへ遷移させるテスト専用の遷移
- Workflow 実行の中断理由と再開位置の項目。domain、usecase DTO、wire protocol（Rust 型・proto・生成 TypeScript 型・手書き TypeScript 型）、presenter、CLI 出力、CLI ガイドのすべて
- `workflow_executions/*.json` への Workflow 実行状態のファイル永続化一式
- `ExecutionStore` の、本番の実行経路から呼ばれないメソッドと、それらだけを検証するテスト
- `recovery_effect_suppression`
- `workflow_execution_logs/` を現在の実装として説明するコメント
- Workflow 実行ストアの責務を説明するコメントのうち、本 ISSUE の削除によって実体と食い違うようになった記述

## 変更しない対象

- NodeExecution が所有する状態（`WaitingApproval` / `Paused` / `Failed` など）と、Node 単位の `can_stop` / `can_resume` / Retry。`docs/glossary/DOMAIN.md` の状態所有に従い、Workflow 実行の状態だけを対象にする
- 利用者の Stop / Resume 操作（`stop_workflow` / `resume_workflow`）と、それが変える Node の状態。操作そのものの廃止と Session の Resume・Command の Retry への整理は、マイルストーン #90 の #1839 が扱う
- `docs/guide/workflow/concepts.md` の「利用者の操作」。Stop / Resume 操作は本 ISSUE の後も残る
- `approval_target`
- 事実ログの形式と、事実ログから Workflow 実行の状態を導出する規則
- 終了時に workflow が起動した command を止める振る舞い
- `shutdown_all_active_commands` と `command_shutdown_intents`。いずれもアプリケーション終了処理から本番経路で呼ばれるため、本 ISSUE の削除基準（本番に到達しない実装）に当たらない

# Requirements

- R-001: Workflow 実行の状態は `Running` / `Completed` / `Aborted` の3つだけである。中断中および承認待ちにあたる Workflow 実行の状態は、テストビルドを含めてどの層にも存在しない
- R-002: Workflow 実行の状態を中断中または承認待ちへ変える状態遷移と、そこから再開する状態遷移は存在しない
- R-003: Workflow 実行の読み取り結果に、中断理由と再開位置の項目が含まれない
- R-004: 取り除いた proto の項目と列挙値の番号・名前は、以後別の意味で再利用されない
- R-005: Workflow 実行の状態をファイルへ保存し、そこから復元する仕組みは存在しない
- R-006: `ExecutionStore` に、本番の実行経路から呼ばれないメソッドは存在しない
- R-007: 値が書き込まれることのない実行時状態は存在しない
- R-008: 読み書きするコードが存在しない記録先を、コメントが現在の実装として説明していない
- R-009: CLI ガイドに、本番で出力されない Workflow 実行の状態値と項目は載っていない
- R-010: Workflow の起動・実行・完了・Abort の振る舞いは変わらない。削除によって出力から消えるのは、本番で常に `null` だった項目と、本番で到達しなかった状態値だけである
- R-011: Workflow 実行ストアの責務を説明するコメントは、本 ISSUE で取り除いた一覧取得とファイル永続化を前提にせず、削除後にそのストアが持つ責務と一致している

# Assumptions / Open Questions

なし。
