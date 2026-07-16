{{ request }} のフルレビュー後修正方針について、Open Thread に残っている承認済み方針 Comment の完全性と相互整合性を確認する。

## 入力

- 全 Open Thread
- 各 Thread の本文、履歴、既存 Comment
- `[FIX_POLICY_APPROVED]` 承認済み修正方針 Comment
- `[FIX_POLICY_CHANGE_REQUEST]` 差し戻し Comment
- タスク（任意の自由文。確認観点の補足指示があれば）: {{ request }}

## 正本ルール

- Thread 状態は `review history` の時系列を正本にする
- Open Thread は、最新の `[FIX_POLICY_CHANGE_REQUEST]` より後に `[FIX_POLICY_APPROVED]` がある場合だけ承認済みとみなす
- `[FIX_POLICY_CHANGE_REQUEST]` が最新の方針状態である Thread は、既存 `[FIX_POLICY_APPROVED]` があっても未完了として扱う
- 未解消の `[FIX_POLICY_CHANGE_REQUEST]` がある限り `LGTM` にしてはならない
- resolved Thread は対象外とし、Open Thread だけを確認する

## プロセス

### 1. Open Thread を取得する

1. `releash review list --session-id "$RELEASH_SESSION_ID" --state open --json` で全 Open Thread を取得する
2. 各 Thread に対して `releash review get <thread-id> --session-id "$RELEASH_SESSION_ID" --json` と `releash review history <thread-id> --session-id "$RELEASH_SESSION_ID" --json` を実行し、本文と履歴を確認する
3. 各 Open Thread について、最新の `[FIX_POLICY_APPROVED]` と `[FIX_POLICY_CHANGE_REQUEST]` を時系列で特定する

Open Thread が 0 件なら、`review-fix-policy-verdict` Artifact として `verdict: "LGTM"` を提出して終了する。

### 2. 全 Open Thread に有効な `[FIX_POLICY_APPROVED]` があるか確認する

各 Open Thread について、最新の `[FIX_POLICY_CHANGE_REQUEST]` より後に `[FIX_POLICY_APPROVED]` Comment があることを確認する。

1 件でも次のいずれかに該当する Thread があれば、その時点で確認結果を報告し、Thread への Comment 投稿は行わずに `review-fix-policy-verdict` Artifact として `verdict: "NEEDS_FIX"` を提出して終了する。後続の完全性・相互整合性チェックには進まない（`decide_policy` 側で `undecided` / `change_requested` として再議論されるため）。

- `[FIX_POLICY_APPROVED]` がそもそも無い
- 最新の `[FIX_POLICY_CHANGE_REQUEST]` より後に `[FIX_POLICY_APPROVED]` が無い（未解消の差し戻し）

### 3. 完全性を確認する

全 Open Thread に有効な `[FIX_POLICY_APPROVED]` がある場合、各 Thread の `[FIX_POLICY_APPROVED]` 内容が次の条件を満たすことを確認する。

- 処理区分、修正方針、受入条件、根拠、対応しない範囲が含まれている
- 修正対象、変更内容、実装範囲、対応しない範囲が明記されており、実装 node が追加判断なしに作業できる
- 受入条件に期待動作、確認観点、必要なテスト観点が明記されており、実装後に方針どおりか判定できる

### 4. 相互整合性を確認する

有効な `[FIX_POLICY_APPROVED]` を横断して、次の不整合がないか確認する。

- 同じファイル、同じ行、同じ機能に対して逆方向の変更を要求している
- ある修正方針を実装すると別 Thread の修正方針または受入条件が成立しなくなる
- テスト期待値、UI 表示、データ形式、エラー条件などの要求が相互に排他になっている
- 実装順序や前提条件が必要なのに、修正方針に明記されていない
- 対応しない範囲が別 Thread の修正方針または受入条件と衝突している

### 5. 問題がある場合

完全性または相互整合性に問題がある Thread には、`[FIX_POLICY_CHANGE_REQUEST]` Comment を投稿し、人間が `decide_policy` で再議論できるようにする。相互整合性問題は Thread を横断する論点なので、関わる Thread のいずれかに投稿する。

既に未解消の `[FIX_POLICY_CHANGE_REQUEST]` が同じ問題で投稿されている場合、同じ内容を重複投稿しない。未解消の差し戻しとして扱う。

Comment は次の形式にする。

```text
[FIX_POLICY_CHANGE_REQUEST]
問題: <内容欠落、矛盾、相互整合性の矛盾内容など>
関連 Thread: <thread-id の一覧>
理由: <なぜこのまま方針として確定できないか>
確認してほしいこと: <人間に決めてほしい論点>
```

問題が 1 件以上ある場合は、`review-fix-policy-verdict` Artifact として `verdict: "NEEDS_FIX"` を提出する。

### 6. 問題がない場合

全 Open Thread が有効な `[FIX_POLICY_APPROVED]` 付きで、未解消の `[FIX_POLICY_CHANGE_REQUEST]` がなく、修正方針同士にも矛盾がなければ、確認結果を簡潔にまとめ、`review-fix-policy-verdict` Artifact として `verdict: "LGTM"` を提出する。

Artifact は次の形で提出する。

```json
{
  "verdict": "LGTM",
  "summary": "確認結果の概要"
}
```

問題がある場合は `verdict` を `"NEEDS_FIX"` にし、`summary` に差し戻し理由の概要を書く。

## 禁止事項

- 実装に着手しない
- `[FIX_POLICY_APPROVED]` Comment を新規投稿・変更しない
- 人間の承認なしに方針を補完しない
- 未解消の `[FIX_POLICY_CHANGE_REQUEST]` がある状態で `verdict: "LGTM"` の Artifact を提出しない
- 方針の矛盾を見つけた Thread を resolve しない
