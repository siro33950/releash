# Design 02

## 開始状態

差分の基準は base ブランチ `main`、派生点 `4267f22f release: v0.4.15 (#1849)`。直前の Design は `docs/specs/issues-1838/design-01.md`。その周の実装が作業ブランチ `feat/issues/1838` に未コミットの変更として入っている状態を、今周の開始状態とする。

`requirements.md` と `behavior.md` は design-01 の周から変わっていない。今周で修正した箇所も無く、Assumptions は「なし」のままである。

この周までに解消・見送りとなった Thread は無い。`[FIX_POLICY]` が付いた open Thread が 4 件（`3a5a5aef-6f1c-4d19-a5b2-f5608f7d5348` / `3de70e2e-20cc-4ff2-abe5-a3faf657fc92` / `890ffa7f-eca6-4557-b6cd-ab2557d83faf` / `59cde2fb-35dd-45b1-8a96-38006d2b81eb`）残っている。`[DEFERRED]`（不採用）と `[REJECTED]`（不成立）にした Thread は無い。

design-01 の「未確定・リスク」に挙げた、enum value を `reserved` にした proto を生成系が期待どおり扱うかは解消した。`proto/client.proto:1690-1691`・`:2990-2991`・`:3235-3236` の `reserved` の後、`src/generated/client_types.ts` と `src/generated/client_pb.ts` に残る `waiting_approval` は `NodeExecutionStatus` 系のものだけで、Workflow 実行の状態から `waiting_approval` / `interrupted` は落ちている。

## 変える部分

- 起動失敗 rollback のコメントを現行の処理に合わせる: `workflow_host.rs:793-798` のコメントから、削除済みの completed 一覧（terminal entry）と metadata 保存の説明を取り除く。根拠: Thread `3a5a5aef-6f1c-4d19-a5b2-f5608f7d5348`（要旨: 起動失敗時のコメントが、本差分で削除した記録先と保存処理を現在の動作として説明している）。R-008「読み書きするコードが存在しない記録先を、コメントが現在の実装として説明していない」/ B-007。ルート: 委任
- `workflow_executions` 不在の検証を、回帰を検出できる形にする: `workflow_host.rs:2694` と `:6232-6236`、`cli/workflow_test.rs:134` の不在検証を、本番の `ExecutionStore` 構成（`new_canonical`）を通る起動・完了・Abort の経路に対する判定にする。根拠: Thread `3de70e2e-20cc-4ff2-abe5-a3faf657fc92`（要旨: 不在 assertion が通る fixture は `new_in_memory_for_tests` を注入しており、削除前の永続化機構のままでも成功するため回帰を検出しない）。R-005「Workflow 実行の状態をファイルへ保存し、そこから復元する仕組みは存在しない」/ B-005、および `docs/architecture/TEST.md`（`adaptor/gateway/` はテスト必須）。ルート: 委任
- 復元時のライフサイクル状態の間接表現を取り除く: `WorkflowExecutionLifecycleRestore`（`domain/workflow/entities/workflow_execution/mod.rs:476-479`）と `lifecycle_from_state`（`:827-829`）を、`RuntimeExecutionState` を包むためだけの型と変換関数が残らない形にする。根拠: Thread `890ffa7f-eca6-4557-b6cd-ab2557d83faf`（要旨: 中断理由の削除後、private な `state` 一つだけを持つ型と、検証せず包んで直後に取り出す変換が残っている）。R-002 と R-003 が要求した削除の帰結。ルート: 委任
- active 集合の登録規則を一箇所にする: `execution_store.rs:131-197` の `register_active_execution` と `:202-263` の `restore_active_snapshot_for_rollback` が、同じ検証・登録規則を共有する形にする。根拠: Thread `59cde2fb-35dd-45b1-8a96-38006d2b81eb`（要旨: ファイル永続化の削除で両者の相違が消え、同じ登録規則の別実装になった）。R-005 が要求した削除の帰結。ルート: 委任

## 固定するルート

今周で新たに固定するルートは無い。design-01 で固定した次の 3 点は今周も維持する。

- proto から項目を取り除くときは番号と名前をともに `reserved` にする（Design 01）
- 削除対象のメソッドは、実装時点で呼び出し元を再確認してから削除する（Design 01）
- `shutdown_all_active_commands` と `command_shutdown_intents` には手を入れない（Design 01）

## 変えないもの

design-01 の「変えないもの」（本番の挙動、`shutdown_all_active_commands` / `command_shutdown_intents`、NodeExecution が所有する状態と利用者の Stop / Resume 操作）を今周も維持する。今周で人間が新たに明示した維持条件は無い。

## 未確定・リスク

- Thread `3de70e2e-20cc-4ff2-abe5-a3faf657fc92` の受入条件「削除したファイル永続化機構を戻した場合に失敗する」を満たす検証手段が未確定である。今の `ExecutionStore`（`execution_store.rs:107-128`）は `data_dir` を持たず、`new_canonical` と `new_in_memory_for_tests` の差は `canonical_query` の有無だけである。本番構成を通す fixture を用意しても、それだけでは削除した機構の回帰を検出できるとは限らない。想定が外れた場合、R-005 / B-005 の検証を満たせない
