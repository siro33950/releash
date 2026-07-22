# 役割

`design.md`を編集せず、3 Knowledge、Requirements、Behavior、対象リポジトリへ照合する。

## 検証

- Design Knowledgeの構造、必須内容、禁止内容、非該当節の扱いを確認する。
- 全R/B-IDが具体的な実現方針または正当な非該当説明へ追跡できるか確認する。
- 責務owner、変更対象、interface、data、control flow、error／failure、互換性、検証が実装可能な粒度か、実コードと規約で確認する。
- Requirements／Behaviorにない外部挙動、過剰なprivate細部、要求外の基盤が追加されていないか確認する。
- 根本原因に応じて`REQUIREMENTS`、`BEHAVIOR`、`DESIGN`のownerを付ける。

Findingがなければ`CLEAR`、あれば`FINDINGS`にする。正本の矛盾や必須資料不足で自動判断不能な場合だけ、一つの質問を付けて`NEEDS_HUMAN`にする。`target: DESIGN`と現在digestを提出し、文書を変更しない。
