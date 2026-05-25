データ:
```json
{
  "policy": "全体方針の自由文（fix ステップが従う全体ガイダンス）",
  "review_step": "code_review_parallel" | "spec_review_parallel" | "bug_investigation" | "bug_review_parallel",
  "findings": [
    {
      "severity": "error" | "warning" | "info",
      "line": "src/foo.ts:42",
      "message": "指摘内容",
      "action": "fix" | "skip",
      "rationale": "この action を選んだ理由"
    }
  ]
}
```

```contract-validation
{
  "result": "approved",
  "required": ["policy", "review_step", "findings"],
  "enums": {
    "review_step": [
      "code_review_parallel",
      "spec_review_parallel",
      "bug_investigation",
      "bug_review_parallel"
    ]
  },
  "array_items_required": {
    "findings": ["severity", "message", "action", "rationale"]
  }
}
```

ルール:
- `policy` は必須・空不可・65536 UTF-8 バイト以下。fix ステップが従う全体方針を自由文で書く
- `review_step` は必須: Spec 修正方針なら `spec_review_parallel`、実装修正方針なら `code_review_parallel`、バグ初回修正方針なら `bug_investigation`、バグ追加修正方針なら `bug_review_parallel`
- `findings` は必須（空配列可）。レビューで挙がった各指摘について判断を1件ずつ記載する
- 各 finding:
  - `severity` 必須: `error` / `warning` / `info`
  - `line` Optional: `<path>:<line>` 形式。レビュー指摘の line をそのまま引き継ぐ
  - `message` 必須: 指摘内容
  - `action` 必須: `fix`（次の fix ステップで対応する）または `skip`（対応しない）
  - `rationale` 必須: なぜその action を選んだか
- Policy が独自に追加する指摘（「ついでに直す」など）も同じ findings 配列に入れて `action: fix` とする
- 後続レビュアーは `action: skip` と判定された指摘を再掲しない
