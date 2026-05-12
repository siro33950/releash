レスポンスに `<workflow_output>` ブロックを必ず1つだけ含めること。

フォーマット:
```text
<workflow_output type="review-verdict">
{
  "verdict": "LGTM" または "NEEDS_FIX",
  "findings": [
    {
      "severity": "error" | "warning" | "info",
      "line": "src/foo.ts:42",
      "message": "説明"
    }
  ]
}
</workflow_output>
```

ルール:
- `verdict` は必須: "LGTM"（問題なし）または "NEEDS_FIX"（問題あり）
- `findings` は verdict が "NEEDS_FIX" の場合に必須（最低1エントリ）
- `findings` は verdict が "LGTM" の場合は空または省略可
- 各 finding には `severity` と `message` が必須
- `line` は Optional: 該当箇所を `<path>:<line>` 形式で指定。ファイル/行が特定できない指摘（全体設計など）は省略可
