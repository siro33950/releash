# 役割

{{project_name}} のフルレビュー後修正について、Open Thread の `[FIX_POLICY_APPROVED]` と現在の実装差分を確認し、未実装・不足・方針不一致を Task にする。

この Step は確認と Task 化だけを行う。コード変更、Thread への Comment 投稿、resolve は行わない。

# 入力

- 環境変数 `RELEASH_SESSION_ID`
- 環境変数 `RELEASH_BASE_BRANCH`
- Open Thread と `[FIX_POLICY_APPROVED]` Comment
- 現在の実装差分

# プロセス

## 1. Open Thread と承認済み方針の取得

次のコマンドで Open Thread を取得する。

```sh
{{path_alias.releash}} review list --session-id "$RELEASH_SESSION_ID" --state open --json
```

必要に応じて各 Thread の詳細と履歴を確認する。

```sh
{{path_alias.releash}} review get <thread-id> --session-id "$RELEASH_SESSION_ID" --json
{{path_alias.releash}} review history <thread-id> --session-id "$RELEASH_SESSION_ID" --json
```

対象は `[FIX_POLICY_APPROVED]` Comment が存在する Open Thread のみ。

## 2. 実装差分の確認

次のコマンドで現在の実装差分を確認する。

```sh
git diff "$(git merge-base "$RELEASH_BASE_BRANCH" HEAD)" HEAD
```

## 3. 実装済み判定

各対象 Thread について、承認済み方針・受入条件・対応しない範囲を逐条確認する。

確認すること:

- 修正方針が実装されているか。
- 受入条件がすべて満たされているか。
- 対応しない範囲に含まれる変更をしていないか。
- 方針外の変更で挙動を変えていないか。
- 必要なテストまたは検証が行われているか。

`[FIX_POLICY_APPROVED]` の妥当性はレビューしない。方針は合意済みとして扱う。

## 4. Task 化

未実装・不足・方針不一致がある場合だけ Task を作る。

Task は、次の実装 Step が Thread を読まずに修正できる粒度で書く。

Task には少なくとも次を含める。

- 元 Thread ID
- 対象ファイルと行番号
- 不足している実装内容
- 満たされていない受入条件
- 対応しない範囲
- 現状と期待状態

重要:

- 方針や受入条件を簡略化しない。
- 独自の方針を追加しない。
- 承認済み方針と無関係な改善を Task 化しない。
- 複数の受入条件がある場合は、同じ Task の `acceptance_criteria` に列挙する。

# 出力

出力Contract `review-fix-tasks` に従って提出する。

不足がない場合:

- `verdict: "LGTM"`
- `tasks: []`
- `summary` に確認結果を書く

不足がある場合:

- `verdict: "NEEDS_FIX"`
- `tasks` に次回実装すべき Task を 1 件以上入れる
- `summary` に不足内容の概要を書く

# 禁止事項

- コード変更を行わない。
- Thread への Comment 投稿、resolve、状態変更を行わない。
- `[FIX_POLICY_APPROVED]` の方針自体の妥当性をレビューしない。
- 承認済み方針を要約だけで置き換えない。
