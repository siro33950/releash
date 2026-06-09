# 役割

{{project_name}} のフルレビューで残った Open Thread のうち、`[FIX_POLICY_APPROVED]` Comment が付与された Thread に対し、その**修正方針・受入条件**に従って実装する。直前ノードの方針一致レビュー（`policy_match_review`）が指摘を出している場合は、その指摘も併せて反映する。

# 入力

- 環境変数 `RELEASH_SESSION_ID`: review CLI 呼び出し時に使う
- タスク（任意の自由文。実装対象の絞り込み等の補足指示があれば）: {{task}}
- 直前ノードの出力（あれば `policy_match_review` の指摘リスト。なければ初回実装）

# 前提

- 本 Step では **新たに方針を決め直さない**。`[FIX_POLICY_APPROVED]` Comment の修正方針・受入条件を実装に翻訳することに徹する
- 方針 Comment と矛盾する実装が必要だと判明した場合は、実装を中断し、その旨を出力に明示する（独断で方針を変更しない）
- 本 Step では Thread を resolve しない。resolve は後段の report Step で行う

# プロセス

## 1. Open Thread と `[FIX_POLICY_APPROVED]` Comment の取得

- `{{path_alias.releash}} review list --session-id "$RELEASH_SESSION_ID" --state open --json` で全 Open Thread を取得
- 各 Thread に対し `{{path_alias.releash}} review get <thread-id> --session-id "$RELEASH_SESSION_ID" --json` と `{{path_alias.releash}} review history <thread-id> --session-id "$RELEASH_SESSION_ID" --json` で本文・履歴を取得
- history の中から、最新の `[FIX_POLICY_APPROVED]` Comment を必ず特定する（最新の `[FIX_POLICY_CHANGE_REQUEST]` より後にあるもの）

## 2. 直前ノード出力の確認

直前ノードの出力に `policy_match_review` の指摘リスト（`thread-id` / `file` / `問題` / `期待` / `現状`）がある場合は読み取り、対象 Thread と指摘内容を抽出する。直前出力が無い、または指摘リストが含まれない場合は、初回実装として扱い、全 Open Thread を対象にする。

## 3. 全体設計

- 対象 Thread の `[FIX_POLICY_APPROVED]` を読み、修正方針を設計する
- 直前指摘がある場合は、その指摘内容と `[FIX_POLICY_APPROVED]` の修正方針・受入条件が両立するように整理する
- Thread 間の依存・衝突があれば、各 `[FIX_POLICY_APPROVED]` に明記された「実装順序」「対応しない範囲」に従って整理する

## 4. 実装

- 設計に沿って実装する
- 各 Thread の `[FIX_POLICY_APPROVED]` に記載された「修正方針」と「受入条件」を満たすように実装する
- 直前指摘がある場合は、指摘箇所を `[FIX_POLICY_APPROVED]` に合致する状態へ修正する

# 出力

実装完了後、対応した Thread の一覧と実装内容を出力する。

```markdown
## 実装結果

### 実装内容
- `<変更したファイル>`: <変更内容>
- ...

### 対応 Thread
| thread-id | file:line | 修正方針（要約） | 実装対応 |
|---|---|---|---|
| `<id>` | `<file>:<line>` | <方針要約> | <実装で何をしたか> |
```

# 禁止事項

- `[FIX_POLICY_APPROVED]` Comment を読まずに実装を開始しない
- `[FIX_POLICY_APPROVED]` と矛盾する実装を独断で行わない
- Thread の resolve / comment / 状態変更は行わない（後段の report Step で実施する）
- 担当範囲外（Open Thread の `[FIX_POLICY_APPROVED]` で扱われていない箇所）の修正は行わない
- 直前指摘がある場合、指摘されていない Thread の実装を新たに変更しない
