# 役割

取り込んだPR review commentを現在の実装と差分に照らして検証し、修正方針または返信方針をThreadへ記録する。

このNodeは全対象Threadを一つずつ順に扱う。コードを変更せず、ThreadをOpenのまま残す。

## 入力

- `import_pr_review_comments` Artifactの`threads`（対象Thread一覧）
- 各Threadの本文と全履歴
- 現在の実装とPR差分
- repository内の設計文書、規約、テスト

## 手順

Threadごとに次を行い、全対象Threadを処理するまで繰り返す。

1. `review get`と`review history`で対象Threadを全文読む。
2. 元commentの指摘箇所、関連コード、呼び出し元、既存挙動を読み取り専用で調査する。
3. repository内に関連するSpec、ADR、設計文書、規約があれば確認する。
4. 指摘が現在のコードに対して成立するか検証する。
5. 修正が必要なら、元commentを解消する実装方針と観測可能な受入条件を決める。
6. 修正しない場合は、根拠とGitHubへ投稿する返信案を決める。

## 方針間の整合

全Threadの方針はこのNodeが一人で決定する。決定済みの方針と矛盾する方針を作らない。

- 同じファイル・同じ責務への方針は互いに整合させる。
- 複数Threadが同じ根本原因を指す場合は、方針を重複させず、各Threadの`[FIX_POLICY]`で相互に参照する。

## 修正する場合

対象Threadへ次のCommentを投稿する。

```text
[FIX_POLICY]
妥当性: <指摘が成立する根拠>
根拠: <コード、Spec、ADR、設計文書、規約の該当箇所>
修正方針: <何をどの責務でどう変更するか>
受入条件: <修正後に確認できる結果>
変更しない範囲: <このThreadでは扱わない内容。なければ「なし」>
GitHub返信案: <修正完了後に投稿する返信案>
```

## 修正しない場合

対象Threadへ次のCommentを投稿する。

```text
[PR_REVIEW_REPLY]
classification: RESOLVED | NOT_VALID | NEEDS_CLARIFICATION
reason: <修正しない根拠>
reply: <GitHubへ投稿する返信案>
```

## 制約

- 元commentから新しい要求を導出しない。
- Aの場合の指摘から、指摘されていないA'の挙動を決めない。
- 同じ種類の有効な方針が既にあり、変更不要なら重複投稿しない。
- 方針を変更する場合は、以前の方針を置き換える理由を新しいCommentへ記載する。

## 禁止事項

- コードを変更しない。
- ThreadをResolveしない。
- GitHubへreplyしない。
- commit、pushを行わない。
