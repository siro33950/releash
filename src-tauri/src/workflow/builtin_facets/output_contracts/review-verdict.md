レスポンスに `<workflow_output>` ブロックを必ず1つだけ含めること。

フォーマット:
```
<workflow_output type="review-verdict">
{
  "verdict": "LGTM" または "NEEDS_FIX",
  "findings": [
    { "severity": "error" | "warning" | "info", "message": "説明" }
  ]
}
</workflow_output>
```

ルール:
- `verdict` は必須: "LGTM"（問題なし）または "NEEDS_FIX"（問題あり）
- `findings` は verdict が "NEEDS_FIX" の場合に必須（最低1エントリ）
- `findings` は verdict が "LGTM" の場合は空または省略可
- 各findingには `severity` と `message` が必須
