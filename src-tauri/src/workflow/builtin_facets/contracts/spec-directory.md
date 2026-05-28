データ:
```json
{
  "spec_dir": "docs/specs/issues-NNN"
}
```

```contract-validation
{
  "required": ["spec_dir"],
  "relative_paths": ["spec_dir"]
}
```

ルール:
- `spec_dir` は必須: Spec ディレクトリへの相対パス
- Spec ディレクトリは `requirements.md`、`behavior.md`、`design.md` を含む
- パスはリポジトリルートからの相対パスとする
- 絶対パス、`..`、末尾の区切り文字 (`/` または `\`) は禁止
