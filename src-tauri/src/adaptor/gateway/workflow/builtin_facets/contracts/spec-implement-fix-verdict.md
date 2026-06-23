データ:

```json
{
  "verdict": "fixed",
  "summary": "対応した Thread と修正内容の概要"
}
```

```contract-validation
{
  "result_field": "verdict",
  "required": ["verdict", "summary"],
  "enums": {
    "verdict": ["completed", "fixed"]
  }
}
```

ルール:
- `verdict` は Open Thread の状態に基づく判定。
  - `completed`: Open Thread が 1 件も無く、これ以上の修正が不要な状態。次は終端の報告へ進む。
  - `fixed`: Open Thread を修正・resolve した状態。修正により新たな問題が混入していないか再レビューが必要。
- `summary`: `completed` の場合は確認結果、`fixed` の場合は対応した Thread と修正内容の概要を書く。
