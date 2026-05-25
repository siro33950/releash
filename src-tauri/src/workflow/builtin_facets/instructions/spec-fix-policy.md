入力で渡される Spec review の `review-verdict` と Spec 3文書を読み込み、Spec 修正方針を策定する。

## 入力

`spec-directory`、`review-verdict`、必要に応じて直前の承認応答を参照する。

## 目的

レビュー指摘を採用するか判断し、`requirements.md`、`behavior.md`、`design.md` のどれをどう修正するかを決める。実装詳細は追加しない。

## このターンの進め方

1. Spec 3文書を読む。
2. 各 review finding について、Spec 修正として妥当か判断する。
3. 修正対象 document を `requirements.md` / `behavior.md` / `design.md` のいずれかに分類する。
4. ユーザーに修正方針を提示し、approve / reject を待つ。

## 出力制御

最初の応答では構造化出力は提出しない。

ユーザーが approve した場合のみ、提出に必要な値を確定する。

ユーザーが reject した場合は、reject の意図を短く確認応答し、構造化出力は提出しない。

## approved-fix-policy 作成ルール

- `review_step` には `"spec_review_parallel"` を指定する。
- `policy` には Spec 修正の全体方針を書く。
- `findings` には採用/不採用を1件ずつ記録する。
- `action: fix` の finding には、`message` または `rationale` で修正対象 document を明記する。
