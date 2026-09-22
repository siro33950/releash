# Context

- 正本: [#1836 `[workflow] 完了を事実として記録し、定義を読めない実行の「先へ進めない」状態を無くす`](https://github.com/siro33950/releash/issues/1836)
- 関連する変更履歴: [#1742](https://github.com/siro33950/releash/issues/1742)、[#1751](https://github.com/siro33950/releash/issues/1751)、commit [`2467b1d0`](https://github.com/siro33950/releash/commit/2467b1d0)、commit [`a3193720`](https://github.com/siro33950/releash/commit/a3193720)。いずれも workflow 定義形式を変更し、旧形式の保存定義が現行コードで解釈不能になる実例を作った
- 関連する作業境界: [#1826](https://github.com/siro33950/releash/issues/1826)、[#1839](https://github.com/siro33950/releash/issues/1839)、[#1840](https://github.com/siro33950/releash/issues/1840)、`docs/specs/issues-1839/requirements.md`、`docs/specs/issues-1839/behavior.md`
- ドメイン語彙と状態所有の正本: `docs/glossary/DOMAIN.md`。WorkflowExecution は `Running` / `Completed` / `Aborted` を所有し、状態遷移は workflow aggregate が決める
- 現行実装の確認先: `src-tauri/src/domain/workflow/value_objects/node_fact.rs`、`src-tauri/src/domain/workflow/services/fact_replay.rs`、`src-tauri/src/adaptor/gateway/workflow/stored_definition.rs`、`src-tauri/src/domain/workflow/entities/workflow_execution/recovery.rs`、`src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs`、`src-tauri/src/domain/workflow/value_objects/node_execution.rs`、`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs`
- 最初の周の調査基準は branch `feat/issues/1836` の `b642f94d`（`origin/main` と同一）である
- workflow の永続化は事実ログであり、自然完了は現行コードでは事実として保存されない。Abort は `abort_requested` として保存され、定義を解釈できない場合も終端状態を維持する

# Outcome

対象者は、Releash で workflow を実行する利用者と、保存済みの実行を運用・保守する開発者である。

現在、自然に完了した実行木には完了の事実が残らず、読み取りのたびに保存定義を現行コードで解釈して完了状態を再計算する。このため、定義形式の変更後に保存定義を解釈できなくなると、以前は完了と計算できた実行が `Running` / `Unresolved` に変わり、進行も終了もできないまま同じ Worktree の次の workflow 起動を妨げる。

変更後は、実行木の完了を事実として記録し、その事実だけで `Completed` と判定することで、終わった実行の状態を後から変化させない。完了またはAbortの事実がなく、保存定義を解釈できない実行は、起動時に理由付きでAbortして終端状態へ収束させ、定義を解釈できないことを理由に進行不能な状態を残さない。

# Current Behavior

- `NodeFact` には開始、プロセス終了、Submit、承認、Retry、Resume、Abort、Archive、Restoreなどがあるが、実行木の完了を表す事実はない
- live経路はroot Nodeが完了するとWorkflowExecutionを `Completed` へ遷移させて `ExecutionCompleted` を生成するが、事実ログへの写像はこのイベントを破棄する
- 読み取り時はrootの `started` に保存したWorkflowDefinitionを現行コードで解釈し、全事実をfoldして完了と次のNodeを再導出する。保存定義は解釈できるNodeだけを組み立て、解釈不能箇所と依存先を `DefinitionResolution`、`recovery_reason`、`Unresolved` で表す
- `Unresolved` はactiveとして扱われる一方、そこから進行または終了する遷移はない
- Abortは `abort_requested` を事実ログへ記録し、fold時に保存定義の解釈結果より優先して `Aborted` を導出する
- 最小の再現は、旧形式の定義で実行木を自然完了させ、完了事実がないまま保存定義と互換性のない版へ更新し、その実行を再読込することである。正本Issueの2026-09-20の実測では、実際の解釈エラーが `invalid type: string "approval", expected a completion map` または ``unknown field `output`, expected `entry` or `children` ``となり、対象実行は `running`、該当Nodeは `unresolved` として出力された。同じWorktreeで後続のworkflowを起動すると、実行中の実行があるとして拒否された

# Scope / Non-goals

## 変更する対象

- 実行木が自然完了したことの事実ログへの記録
- 完了事実を持つ実行の、保存定義に依存しない終端状態の判定
- 終端事実を持ち、保存定義を現行コードで解釈できる既存実行について、NodeExecutionの公開情報の互換維持
- 完了またはAbortの事実がなく、保存定義を現行コードで解釈できない実行を、起動時に理由付きでAbortする処理
- 保存定義を解釈できないために `Unresolved` となり、進行不能なまま `Running` に留まる状態と、それを支える部分的な定義解釈・依存判定
- 既に `Running` に戻っている保存済み実行を、完了と推定せず理由付きAbortへ収束させること

## 変更しない対象

- 旧形式の保存定義を現行形式へ移行すること、および旧形式を継続して解釈する互換経路
- 完了事実を持たない既存実行に対し、過去の自然完了を推定して完了事実を補うこと
- 保存定義を現行コードで解釈できず、durable workflow factだけでは区別できない既存終端実行のNodeExecution情報を推定すること
- Archiveの記録統合、実行中の実行をArchiveするときのAbort、消えたWorktreeの実行の片付け。これらは #1826 が扱う
- 起動時処理全体とメモリ上のWorkflow状態の整理。保存定義を解釈できない未完了実行のAbort以外は #1840 が扱う

# Requirements

- R-001: 実行木が完了したとき、その完了がdurable workflow factとして事実ログへ記録される
- R-002: 完了の事実を持つWorkflowExecutionは、保存定義を解釈せずに `Completed` と判定され、後の定義形式または実行コードの変更によって終端状態が変化しない
- R-003: 完了またはAbortの事実を持たない実行の保存定義を現行コードで解釈できない場合、起動時の処理は定義を解釈できない理由を伴うAbortの事実を記録し、その実行を `Aborted` にする
- R-004: 完了の事実がなく、保存定義の非互換によって既に `Running` に戻っている既存実行は、起動時にR-003と同じ理由付きAbortで終了する
- R-005: 保存定義を解釈できないことを理由とする `Unresolved` のNode状態、および利用者が進行も終了もできない `Running` のWorkflowExecutionは存在しない
- R-006: 完了またはAbortの終端事実を持つ既存実行で、保存定義を現行コードで解釈できる場合、同じ事実列と保存定義から得られるNodeExecutionの公開情報は変更前から変化しない

# Assumptions / Open Questions

- 自動判断: Thread `90b1c19a-4d27-4fb9-a175-4da7a9659a87`について、R-002の定義非依存性はWorkflowExecutionの終端状態判定に限定する。保存定義を解釈できる既存終端実行ではNodeExecutionの公開情報を互換維持し、保存定義を解釈できずdurable workflow factだけでは区別できないNodeExecution情報は推定しない。元要求が固定している終端状態の不変性を満たしつつ、既存挙動を維持し、新しい観測可能な結果を追加しない最小の解釈である
