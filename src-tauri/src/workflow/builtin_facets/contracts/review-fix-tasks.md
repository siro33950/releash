データ:

```json
{
  "verdict": "NEEDS_FIX",
  "tasks": [
    {
      "task_id": "task-001",
      "thread_id": "<thread-id>",
      "file": "src/foo.rs:120",
      "objective": "不足している実装を完了する",
      "acceptance_criteria": [
        "満たされていない受入条件を列挙する"
      ],
      "non_goals": [
        "対応しない範囲を列挙する"
      ],
      "source_policy": "FIX_POLICY_APPROVED の該当箇所",
      "problem": "方針・受入条件に対する不足内容",
      "expected": "期待される実装状態",
      "actual": "現在の実装状態"
    }
  ],
  "summary": "不足内容の概要"
}
```

```contract-validation
{
  "result_field": "verdict",
  "required": ["verdict", "tasks", "summary"],
  "enums": {
    "verdict": ["LGTM", "NEEDS_FIX"]
  },
  "non_empty_array_when": [
    {
      "field": "verdict",
      "equals": "NEEDS_FIX",
      "array": "tasks"
    }
  ],
  "array_items_required": {
    "tasks": ["task_id", "thread_id", "file", "objective", "source_policy", "problem", "expected", "actual"]
  }
}
```

ルール:
- `verdict: "LGTM"` の場合、`tasks: []` とする。
- `verdict: "NEEDS_FIX"` の場合、`tasks` に次回実装すべき Task を 1 件以上入れる。
- `tasks[].task_id`: workflow 内で参照できる安定した Task ID。
- `tasks[].thread_id`: 元 Thread ID。
- `tasks[].file`: 対象ファイルと行番号。複数箇所にまたがる場合は主対象を書く。
- `tasks[].objective`: 次の実装担当が行うべきこと。
- `tasks[].acceptance_criteria`: 満たされていない受入条件を省略せず転記する。
- `tasks[].non_goals`: 対応しない範囲がある場合に転記する。なければ空配列。
- `tasks[].source_policy`: Task の根拠になった承認済み方針。
- `tasks[].problem`: 方針・受入条件に対する不足内容。
- `tasks[].expected`: 方針・受入条件に基づく期待状態。
- `tasks[].actual`: 実コードの現在状態。
