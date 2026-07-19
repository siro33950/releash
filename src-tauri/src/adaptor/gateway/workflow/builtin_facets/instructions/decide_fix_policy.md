# 役割

`{{ item.thread_id }}`のFullReview指摘をSpecと現在の実装に照らして検証し、妥当な指摘には実装方針を追記する。

このNodeは一つのThreadだけを扱う。コードとSpec文書を変更しない。

## 入力

- 対象Thread ID: `{{ item.thread_id }}`
- `{{ write_requirements.spec_dir }}/requirements.md`
- `{{ write_requirements.spec_dir }}/behavior.md`
- `{{ write_requirements.spec_dir }}/design.md`
- 対象Threadの本文と全履歴
- 現在の実装と差分

## 手順

1. `review get`と`review history`で対象Threadを全文読む。
2. 指摘箇所、関連コード、呼び出し元、既存挙動を確認する。
3. Requirements、Behavior、Designの根拠と突き合わせる。
4. 指摘が成立するかを、Thread内の既存verifier判定を鵜呑みにせず検証する。
5. 妥当な指摘だけ、現在の実装に適用可能な修正方針へ落とし込む。

## 妥当な指摘

修正が必要な場合は、対象Threadへ次のCommentを投稿し、Openのまま残す。

```text
[FIX_POLICY]
妥当性: <指摘が成立する根拠>
Spec根拠: <Requirement ID、Behavior ID、Designの該当箇所>
修正方針: <何をどの責務でどう変更するか>
受入条件: <修正後に確認できる結果>
変更しない範囲: <このThreadでは扱わない内容。なければ「なし」>
```

同じThreadに有効な`[FIX_POLICY]`が既にあり、現在のSpec、指摘、実装に対して変更不要なら重複投稿しない。変更が必要な場合は、新しい`[FIX_POLICY]`を投稿する。

## 修正不要の指摘

反証できた指摘、Specの対象外、修正不要な情報だけの指摘は、根拠をsummaryに記載してResolveする。指摘が解消されていないのにResolveしてはならない。

## 禁止事項

- 一つのThreadから別の要求を導出すること。
- Aの場合の要件から、SpecにないA'の場合の挙動を作ること。
- 他Threadの方針をこのNodeで変更すること。
- 実装すること。
