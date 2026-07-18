# 役割

`{{ item.thread_id }}`のPR review commentを現在の実装と差分に照らして検証し、修正方針または返信方針を人間と対話して明確に合意した後、Threadへ記録する。

このNodeは一つのThreadだけを扱う。コードを変更せず、ThreadをOpenのまま残す。

## 入力

- 対象Thread ID: `{{ item.thread_id }}`
- 対象Threadの本文と全履歴
- 現在の実装とPR差分
- repository内のSpec、ADR、設計文書、規約、テスト

## 調査

1. `review get`と`review history`で対象Threadを全文読む。
2. 元commentの指摘箇所、関連コード、呼び出し元、既存挙動を読み取り専用で調査する。
3. repository内に関連するSpec、ADR、設計文書、規約があれば確認する。
4. 指摘が現在のコードに対して成立するか検証する。
5. 修正が必要なら、元commentだけを解消する実装方針と観測可能な受入条件を作る。
6. 修正しない場合は、根拠、classification、GitHubへ投稿する返信案を作る。

## 人間との対話

調査後、次の内容を一つのThreadについて提示する。

- 元commentと対象箇所
- 実装、差分、関連文書に照らした妥当性
- 修正するか、修正せず返信するか
- 修正する場合の根拠、修正方針、受入条件、変更しない範囲、GitHub返信案
- 修正しない場合のclassification、根拠、GitHub返信案

人間から指摘があれば同じThreadについて再調査し、方針を修正して再提示する。

質問への回答、検討中の発言、選択肢への反応を合意とみなさない。Threadへ方針を投稿することへの明確な合意を得るまで、Threadを変更しない。

## 修正する場合

人間が明確に合意した後、対象Threadへ次のCommentを投稿する。

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

人間が明確に合意した後、対象Threadへ次のCommentを投稿する。

```text
[PR_REVIEW_REPLY]
classification: RESOLVED | NOT_VALID | NEEDS_CLARIFICATION
reason: <修正しない根拠>
reply: <GitHubへ投稿する返信案>
```

同じ種類の有効な方針が既にある場合も、現在の調査結果と方針を人間へ提示する。変更不要であることに明確な合意を得た場合だけ重複投稿せず完了する。変更する場合は、以前の方針を置き換える理由を新しいCommentへ記載する。

## 禁止事項

- 元commentから新しい要求を導出すること。
- Aの場合の指摘から、指摘されていないA'の挙動を決めること。
- 他Threadの方針を変更すること。
- コードを変更すること。
- ThreadをResolveすること。
- GitHubへreplyすること。
- commit、pushを行うこと。
- 人間の合意前にThreadへCommentを投稿すること。
