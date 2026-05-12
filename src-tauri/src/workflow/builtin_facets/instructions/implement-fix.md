{{project_name}} プロジェクトのコードを承認済み実装修正方針に基づいて修正する。

## 入力

- ユーザーがApproveした実装修正方針（`approved-fix-policy`、ステップ出力経由）
- Specファイルパス（`plan_requirements` の出力経由）— 参照用

## 早期終了判定（最優先）

`approved-fix-policy.findings` の **全件が `action: "skip"`** の場合、ファイル編集を一切行わず、レスポンス末尾に厳密に以下の1行のみを出力して終了する:

```
NO_FIX_NEEDED
```

前後に説明文を一切付けない。この1トークンがワークフローのルーティングキーとして使われる。

## プロセス（`action: "fix"` の指摘が1件以上ある場合）

1. `approved-fix-policy.findings` の各要素を参照し、`action: "fix"` のものだけを修正対象とする
2. 必要に応じて Spec ファイル（`spec_file_path`）を読み込み、要求と振る舞い定義のコンテキストを把握する
3. 各修正対象について:
   - `line`（指定があれば）を参考に該当コードを読み込む
   - `rationale` と `policy` 全体方針に沿う修正だけを適用する
   - 根本原因に対処する
   - 修正が既存の振る舞いを壊さないことを確認する
4. `action: "skip"` の指摘は対象外（編集しない）
5. 各修正後に関連テストを実行する
6. lint/フォーマットチェックを実行する

## 修正方針

- `approved-fix-policy.findings` を正本とし、レビュー指摘だけから修正方針を推測しない
- severity "error" の `action: "fix"` 指摘を先に対処する
- 指摘への対処に必要な範囲を超えた変更は行わない
- 修正提案が振る舞い定義のScenarioに違反する場合は、指摘とScenarioの両方を満たす代替修正を適用する
- 全修正完了後に品質チェック（lint、test）を実行する
