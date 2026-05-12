---
type: instruction
key: plan-fix-policy
description: 人間向けに修正方針を報告し、承認を得てから approved-fix-policy を出力する
---

# Plan Fix Policy Approval

`plan_review_parallel` から渡されたレビュー結果（4観点 reviewer の `review-verdict`）と、Spec ファイル（`plan_requirements` の出力経由）を読み込み、Plan の修正方針を策定する。

ファイル編集は一切行わない。

## 動作プロトコル

### ターン1（初回応答）

レビュー結果を分析し、**人間が読める形式**で以下を報告する:

1. **全体方針サマリ**: 修正の優先順位や全体の考え方を短く（2〜4 行）
2. **指摘ごとの判断**: 各レビュー findings について、表形式または箇条書きで:
   - severity / line（あれば） / message
   - 提案する `action`: `fix` または `skip`
   - 理由（`rationale`）
3. **Policy 独自に追加する指摘**（任意）: 「ついでに直す」項目があれば同じ形式で
4. 最後にユーザーへ承認を求める:

   > この方針で承認しますか? 変更があれば指示してください。

**このターンでは `<workflow_output>` ブロックを絶対に出力しない。**

### ターン2以降

ユーザーの応答に応じて分岐する:

- **承認された場合**（「OK」「承認」「approve」「いいよ」等）:
  下記フォーマットの `<workflow_output>` ブロックを**1つだけ**出力する。前後に文章を付けない。

- **修正指示があった場合**:
  指示に従って方針を更新し、再度ターン1の構成で人間向けに報告し直して承認を求める。

- **却下（reject）された場合**:
  `reject` の意図を1行で確認応答する（`<workflow_output>` は出力しない）。ワークフローエンジンが `match: reject` ルールで分岐する。

## `<workflow_output>` フォーマット（承認後のみ）

```text
<workflow_output type="approved-fix-policy">
{
  "policy": "承認された全体方針（自由文）",
  "review_step": "plan_review_parallel",
  "findings": [
    {
      "severity": "error" | "warning" | "info",
      "line": "<path>:<line>",
      "message": "指摘内容",
      "action": "fix" | "skip",
      "rationale": "判断理由"
    }
  ]
}
</workflow_output>
```

- `findings` は人間に報告した内容と完全に一致させる
- `line` は元の review-verdict から引き継ぐ（無い場合は省略）
- 追加項目（review にない「ついでに直す」など）も同じ配列に入れる
