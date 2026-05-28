データ:
```json
{
  "symptom": {
    "current_behavior": "現在の挙動",
    "expected_behavior": "期待する挙動",
    "repro_steps": "再現手順",
    "impact_scope": "影響範囲（ユーザー / 機能 / データ）"
  },
  "related_code": [
    { "path": "<path>", "line": <number>, "role": "役割の説明" }
  ],
  "root_cause": {
    "class": "state_transition" | "boundary" | "exception_swallow" | "type_mismatch" | "concurrency" | "config" | "other",
    "description": "コード上の根拠を引用した説明"
  },
  "fix_candidates": [
    {
      "name": "候補名",
      "summary": "概要",
      "expected_effect": "期待される効果",
      "side_effects": "想定影響"
    }
  ],
  "impact": ["既存テストで守られている振る舞いへの影響", "..."]
}
```

```contract-validation
{
  "required": ["symptom", "related_code", "root_cause", "fix_candidates", "impact"],
  "array_items_required": {
    "related_code": ["path", "role"],
    "fix_candidates": ["name", "summary", "expected_effect", "side_effects"]
  }
}
```

ルール:
- `symptom.current_behavior` / `symptom.expected_behavior` は必須
- `symptom.repro_steps` は明示されていれば必須、不明なら省略可
- `related_code` は最低1件。`line` は特定できる場合のみ
- `root_cause.class` は必須。原因クラスのいずれかを選ぶ
- `root_cause.description` は必須。コード上の根拠を引用しながら説明する
- `fix_candidates` は最低1件
- `impact` は影響を予測した上で配列で列挙する（空配列可）
