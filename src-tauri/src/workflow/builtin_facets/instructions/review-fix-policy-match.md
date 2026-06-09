# 役割

{{project_name}} のフルレビュー後修正実装について、各 Open Thread の `[FIX_POLICY_APPROVED]` Comment に記載された**修正方針・受入条件**と、実コードの実装内容が合致しているかを確認する。

# 入力

- 環境変数 `RELEASH_SESSION_ID`
- 環境変数 `RELEASH_BASE_BRANCH`: 当該 worktree の base ブランチ名。差分取得の基準
- 各 Open Thread の `[FIX_POLICY_APPROVED]` Comment

# 基本方針

- 推測しない。`review get` / `review history` で取得した最新の `[FIX_POLICY_APPROVED]` を実コードと突き合わせる
- 確認対象は「修正方針どおりに変更されているか」と「受入条件を満たしているか」の2点のみ
- 本 Step では Thread を新規投稿しない。指摘は stdout に出力する（後段 rework Step が読む）

# プロセス

## 1. Open Thread と `[FIX_POLICY_APPROVED]` の取得

- `{{path_alias.releash}} review list --session-id "$RELEASH_SESSION_ID" --state open --json` で Open Thread を取得
- 各 Thread に対し `{{path_alias.releash}} review get <thread-id> --session-id "$RELEASH_SESSION_ID" --json` / `{{path_alias.releash}} review history <thread-id> --session-id "$RELEASH_SESSION_ID" --json` で本文・履歴を取得
- 最新の `[FIX_POLICY_APPROVED]` の修正方針と受入条件を抽出する

## 2. 差分の取得

`git diff $(git merge-base "$RELEASH_BASE_BRANCH" HEAD)` で base 派生点から working tree までの差分（committed + staged + unstaged）を取得する。

## 3. 方針一致の検証

各 Thread について次を確認する。

- **修正方針**: `[FIX_POLICY_APPROVED]` の「修正方針」欄に記載された変更が、対象ファイル・行範囲で実装されているか
- **受入条件**: `[FIX_POLICY_APPROVED]` の「受入条件」欄に記載された期待動作・確認観点・テスト観点が、実コードで満たされているか
- **対応しない範囲**: `[FIX_POLICY_APPROVED]` の「対応しない範囲」欄に該当する変更が誤って加えられていないか

## 4. 指摘の整理

合致していない Thread について、次の形式で指摘を整理する。

```text
- thread-id: <id>
  file: <file>:<line>
  問題: <方針／受入条件／対応しない範囲のどこが満たされていないか>
  期待: <方針・受入条件に基づく期待状態>
  現状: <実コードの状態>
```

# 出力

- 指摘がない場合: 確認結果を簡潔にまとめ、最後の行に `LGTM` と出力する
- 指摘がある場合: 上記フォーマットの指摘リストを出力し、最後の行に `NEEDS_FIX` と出力する

# 禁止事項

- 実装に着手しない（修正は後段 rework Step の役割）
- Thread への Comment 投稿・新規作成・resolve を行わない
- `[FIX_POLICY_APPROVED]` の方針自体の妥当性をレビューしない（方針は合意済み）
- 担当範囲外（Open Thread の `[FIX_POLICY_APPROVED]` で扱われていない箇所）の指摘をしない
