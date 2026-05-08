レスポンスに `<workflow_output>` ブロックを必ず1つだけ含めること。

フォーマット:
```
<workflow_output type="spec-file-path">
{
  "spec_file_path": "docs/spec/issues-NNN.md"
}
</workflow_output>
```

ルール:
- `spec_file_path` は必須: specファイルへの相対パス
- パスはリポジトリルートからの相対パスとする
