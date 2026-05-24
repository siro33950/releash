---
type: instruction
key: bug-fix-policy
description: バグ原因調査の結果を踏まえ、修正方針をユーザーに承認させて approved-fix-policy を CLI 提出する
---

# Bug Fix Policy Approval

入力で渡される根本原因分析（`bug-investigation-result`）と修正候補をもとに、実際に適用する修正方針を策定し、ユーザーの承認を得る。

ファイル編集は一切行わない。

## 動作プロトコル

### ターン1（初回応答）

調査結果を分析し、**人間が読める形式**で以下を報告する:

1. **根本原因サマリ**: 入力の根本原因分析の結論を1〜2行で要約
2. **採用する修正方針**: 複数の修正候補から選択した理由（または独自に追加する方針）を 2〜4 行で
3. **具体的な修正項目リスト**: 表形式または箇条書きで、各修正項目について:
   - 対象: ファイル:行（特定できる場合）
   - 修正内容の要約
   - 提案する `action`: `fix` または `skip`
   - 理由（`rationale`）
4. **影響範囲・リスク**: 既存振る舞いへの影響、テスト戦略を1〜2行
5. 最後にユーザーへ承認を求める:

   > この方針で修正に進みますか? 変更があれば指示してください。

**このターンでは構造化出力は提出しない（`releash workflow output submit` を呼ばない）。**

### ターン2以降

ユーザーの応答に応じて分岐する:

- **承認された場合**（「OK」「承認」「approve」「いいよ」等）:
  下記「承認後の出力」に従い `releash workflow output submit` で `approved-fix-policy` を提出する。

- **修正指示があった場合**:
  指示に従って方針を更新し、再度ターン1の構成で人間向けに報告し直して承認を求める。

- **却下（reject）された場合**:
  `reject` の意図を1行で確認応答する（構造化出力は提出しない）。

## 承認後の出力

ユーザーから承認を得た場合、`approved-fix-policy` Contract に従う JSON を組み立て、当該 step 名と `run_id` を主語に `releash workflow output submit` を呼んで提出する。

```sh
releash workflow output submit <run_id> \
  --step <step_name> \
  --type approved-fix-policy \
  --json '{"review_step":"bug_investigation","findings":[...]}'
```

- `review_step` には `"bug_investigation"` を指定する（本 policy が参照した一次入力の種別）
- `findings` は人間に報告した修正項目と完全に一致させる
- 提出が成功するまで step は完了として扱われない。失敗時は `releash workflow output validate` でフォーマットを確認してから再提出する
