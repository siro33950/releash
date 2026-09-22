# Design 06

## 開始状態

差分の基準は base branch `main` の `b642f94d feat(workflow): プロセス在否に基づくNode再開と起動再試行を導入 (#1860)` で、作業 branch `feat/issues/1836` の派生点も同じ commit である。直前のDesignは `docs/specs/issues-1836/design-05.md` であり、その実装は未コミット差分として存在する。

Design 05 で参照したThread `59fc794d-3e38-4aec-b3f6-7c7f2eb30057` は解消済みである。今周の開始時点では、`[FIX_POLICY]` 付きopen Thread `90b1c19a-4d27-4fb9-a175-4da7a9659a87` と `2930f5df-a03a-41da-ba29-64623879c482` が未解消である。見送りとなったThreadはない。

## 変える部分

- 終端状態判定とNodeExecution公開情報の復元の両立: WorkflowExecutionの終端状態は終端事実から保存定義に依存せず判定し、保存定義を現行コードで解釈できる既存終端実行では、同じ事実列と保存定義から得られるNodeExecutionの公開情報を変更前から維持する。保存定義を解釈できない場合は、durable workflow factだけでは区別できないNodeExecution情報を推定しない。根拠: R-002、R-006、B-002、B-005、Thread `90b1c19a-4d27-4fb9-a175-4da7a9659a87`。ルート: 委任
- 完了事実の書込結果不明時の状態収束: 完了事実のappend結果が不明になった場合にcanonical logから状態を再導出し、完了事実がdurableであれば、同一プロセス内のWorkflowExecutionとactive projectionも `Completed` へ収束させる。根拠: R-002、B-002、Thread `2930f5df-a03a-41da-ba29-64623879c482`。ルート: 委任

## 固定するルート

固定する実装上の指定なし

## 変えないもの

なし

## 未確定・リスク

- 自動判断: Thread `90b1c19a-4d27-4fb9-a175-4da7a9659a87`について、R-002の定義非依存性はWorkflowExecutionの終端状態判定に限定し、保存定義を解釈できる既存終端実行のNodeExecution公開情報は互換維持する。保存定義を解釈できず、durable workflow factだけでは区別できないNodeExecution情報は推定しない
- 未決の要求および`[DEFERRED]`で人間へ渡した事項はない
