レスポンスに `<workflow_output>` ブロックを必ず1つだけ含めること。

フォーマット:
```
<workflow_output type="fix-result">
{
  "status": "FIXED" または "PARTIAL" または "BLOCKED",
  "changes": [
    { "file": "path/to/file", "description": "変更内容" }
  ]
}
</workflow_output>
```

ルール:
- `status` は必須: "FIXED"（全問題解決）、"PARTIAL"（一部解決）、または "BLOCKED"（続行不能）
- `changes` は任意。変更したファイルとその説明のリスト
