# Implementation Fix Policy Approval

入力で渡されるレビュー結果（6観点 reviewer の `review-verdict`）と Spec 3文書を読み込み、実装の修正方針を策定する。

ファイル編集は一切行わない。

## 動作プロトコル

### ターン1（初回応答）

レビュー結果を分析し、**人間が読める形式**で以下を報告する:

1. **全体方針サマリ**: 修正の優先順位（error 先行、テスト/lint 後実行など）や全体の考え方を短く（2〜4 行）
2. **指摘ごとの判断**: 各レビュー findings について、表形式または箇条書きで:
   - severity / line（あれば） / message
   - 提案する `action`: `fix` または `skip`
   - 理由（`rationale`）
3. **Policy 独自に追加する指摘**（任意）: 「ついでに直す」項目があれば同じ形式で
4. 最後にユーザーへ承認を求める:

   > この方針で承認しますか? 変更があれば指示してください。

**このターンでは構造化出力は提出しない（`releash workflow output submit` を呼ばない）。**

### ターン2以降

ユーザーの応答に応じて分岐する:

- **承認された場合**（「OK」「承認」「approve」「いいよ」等）:
  下記「承認後の出力」に従い `releash workflow output submit` で `approved-fix-policy` を提出する。

- **修正指示があった場合**:
  指示に従って方針を更新し、再度ターン1の構成で人間向けに報告し直して承認を求める。

- **却下（reject）された場合**:
  `reject` の意図を1行で確認応答する（構造化出力は提出しない）。ワークフローエンジンが `match: reject` ルールで分岐する。

## 承認後の出力

ユーザーから承認を得た場合、`approved-fix-policy` Contract に従う JSON を組み立て、当該 step 名と `run_id` を主語に `releash workflow output submit` を呼んで提出する。

```sh
releash workflow output submit <run_id> \
  --step <step_name> \
  --type approved-fix-policy \
  --json '{"review_step":"code_review_parallel","findings":[...]}'
```

- `review_step` には `"code_review_parallel"` を指定する（本 policy が参照した一次入力の種別）
- `findings` は人間に報告した内容と完全に一致させる
- 追加項目（review にない「ついでに直す」など）も同じ配列に入れる
- 提出が成功するまで step は完了として扱われない。失敗時は `releash workflow output validate` でフォーマットを確認してから再提出する
