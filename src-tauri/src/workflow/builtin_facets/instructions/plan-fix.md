{{project_name}} プロジェクトのSpecファイル（要求・振る舞い定義・アーキテクチャ概要）を承認済みPlan修正方針に基づいて修正する。

## 入力

- ユーザーがApproveしたPlan修正方針（`approved-fix-policy`、ステップ出力経由）
- Specファイルパス（`plan_requirements` の出力経由）

## 早期終了判定（最優先）

`approved-fix-policy.findings` の **全件が `action: "skip"`** の場合、ファイル編集を一切行わず、レスポンス末尾に厳密に以下の1行のみを出力して終了する:

```
NO_FIX_NEEDED
```

前後に説明文を一切付けない。この1トークンがワークフローのルーティングキーとして使われる。

## プロセス（`action: "fix"` の指摘が1件以上ある場合）

1. Specファイルを読み込む（`plan_requirements` の出力の `spec_file_path`）
2. `approved-fix-policy.findings` の各要素を参照し、`action: "fix"` のものだけを修正対象とする
3. 各修正対象について:
   - Specの修正箇所を特定する（`line` がある場合はそれを参考）
   - `rationale` と `policy` 全体方針に沿う修正だけを適用する
   - 全体構造を維持しながら修正を適用する
   - 表面的な症状ではなく根本原因に対処する
4. `action: "skip"` の指摘は対象外（編集しない）
5. 全ての修正をSpecファイルに反映する

## 修正方針

- `approved-fix-policy.findings` を正本とし、レビュー指摘だけから修正方針を推測しない
- severity "error" の `action: "fix"` 指摘を先に対処する
- 適切な修正が曖昧な指摘には、最も保守的な修正を適用する
- 指摘への対処に必要な範囲を超えた内容追加は行わない
- 既存のSpecのスタイルと用語の一貫性を維持する
