# Design 03

## 開始状態

差分の基準は base ブランチ `main`、派生点 `4267f22f release: v0.4.15 (#1849)`。直前の Design は `docs/specs/issues-1838/design-02.md`。その周の実装が作業ブランチ `feat/issues/1838` に未コミットの変更として入っている状態を、今周の開始状態とする。

`requirements.md` は前段（triage_requirements_and_behavior）が R-011 と Scope / Non-goals の「変更する対象」1 項目を追加した形、`behavior.md` は B-010 と対応表の R-011 行を追加した形である。今周でこの 2 文書を修正した箇所は無く、Assumptions は「なし」のままである。

design-02 の「変える部分」に対応する Thread 4 件（`3a5a5aef-6f1c-4d19-a5b2-f5608f7d5348` / `3de70e2e-20cc-4ff2-abe5-a3faf657fc92` / `890ffa7f-eca6-4557-b6cd-ab2557d83faf` / `59cde2fb-35dd-45b1-8a96-38006d2b81eb`）はいずれも resolve 済みである。design-02 の「未確定・リスク」に挙げた、削除したファイル永続化機構の回帰を検出する検証手段は、本番と同じ `ExecutionStore::new_canonical` を注入する fixture（`workflow_host_test.rs`）が置かれたことで解消した。

`[FIX_POLICY]` が付いた open Thread が 2 件（`32f2c72c-adc4-4e9c-a0dc-be85dffec8e7` / `3ecd97ca-9a9d-454e-ae6c-05e50949a60f`）残っている。`[DEFERRED]`（不採用）と `[REJECTED]`（不成立）にした Thread は無い。

## 変える部分

- Workflow 実行ストアの責務コメントを現在の責務に合わせる: `execution_store.rs:79-84` の `ExecutionStore` の doc comment から、一覧取得の管理（:80「active/list UI projection と SQLite projection authority を管理する」）と、削除済みの filesystem metadata への言及（:84）を取り除き、現在この型が提供する active registry と単一実行の取得に一致させる。根拠: R-011「Workflow 実行ストアの責務を説明するコメントは、本 ISSUE で取り除いた一覧取得とファイル永続化を前提にせず、削除後にそのストアが持つ責務と一致している」/ B-010、Thread `32f2c72c-adc4-4e9c-a0dc-be85dffec8e7`（要旨: 責務コメントが、本差分で削除した一覧取得を前提にした説明のまま残っている）。ルート: 委任
- 実行 summary の二重定義と転記を解消する: `execution_store.rs:19-31` の `WorkflowExecutionMetadata` と `domain/workflow/value_objects/execution_metadata.rs:36-48` の `WorkflowExecutionSummary` が、項目名・型の一致する 11 項目を二重に定義しない形にし、`execution_store.rs:443-459` の `workflow_summary_to_metadata` による項目そのままの転記が残らない形にする。根拠: Thread `3ecd97ca-9a9d-454e-ae6c-05e50949a60f`（要旨: ファイル永続化の削除で外部形式としての役割が消え、同一 summary の別表現と転記だけが残っている）。R-005 / B-005 が要求した削除の帰結であり、R-010「Workflow の起動・実行・完了・Abort の振る舞いは変わらない」/ B-009 のとおり `get_execution_record` を使う `validate_execution_command_target`、active 登録・rollback、CLI / Connect API の出力値は変更前と同じにする。ルート: 委任

## 固定するルート

今周で新たに固定するルートは無い。design-01 で固定した次の 3 点は今周も維持する。

- proto から項目を取り除くときは番号と名前をともに `reserved` にする（Design 01）
- 削除対象のメソッドは、実装時点で呼び出し元を再確認してから削除する（Design 01）
- `shutdown_all_active_commands` と `command_shutdown_intents` には手を入れない（Design 01）

上記 2 件の解消方法は人間が実装側へ委任している。書き直す doc comment の文言、および二重定義をどちらの型へ寄せるか（gateway 側の型を無くすか、domain の型を直接使うか等）は固定しない。

## 変えないもの

design-01 の「変えないもの」（本番の挙動、`shutdown_all_active_commands` / `command_shutdown_intents`、NodeExecution が所有する状態と利用者の Stop / Resume 操作）を今周も維持する。今周で人間が新たに明示した維持条件は無い。

## 未確定・リスク

なし。
