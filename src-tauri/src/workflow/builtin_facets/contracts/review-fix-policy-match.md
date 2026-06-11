データ:
```json
{
  "verdict": "LGTM",
  "findings": []
}
```

```contract-validation
{
  "result_field": "verdict",
  "required": ["verdict", "findings"],
  "enums": {
    "verdict": ["LGTM", "NEEDS_FIX"]
  },
  "non_empty_array_when": [
    {
      "field": "verdict",
      "equals": "NEEDS_FIX",
      "array": "findings"
    }
  ],
  "array_items_required": {
    "findings": ["thread_id", "file", "problem", "expected", "actual"]
  }
}
```

ルール:
- `verdict` は `LGTM` または `NEEDS_FIX`
- 指摘がない場合は `verdict: "LGTM"`、`findings: []`
- 指摘がある場合は `verdict: "NEEDS_FIX"` とし、`findings` に指摘を 1 件以上入れる
- `findings[].thread_id`: 対象 Thread ID
- `findings[].file`: 対象ファイルと行番号（例: `src/foo.rs:120`）
- `findings[].problem`: 方針・受入条件・対応しない範囲のどこが満たされていないか
- `findings[].expected`: 方針・受入条件に基づく期待状態
- `findings[].actual`: 実コードの状態
