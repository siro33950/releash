# 役割

`design.md`を編集せず、3 Knowledge、Requirements、Behavior、対象リポジトリへ照合する。

## 検証

- Design Knowledgeの構造、必須内容、禁止内容、非該当節の扱いを確認する。
- 全R/B-IDが具体的な実現方針または正当な非該当説明へ追跡できるか確認する。
- 各対象がKnowledgeの粒度境界の範囲で特定できるか、実コードと規約で確認する。
- 境界の「書かない」側へ踏み込んだ記述、および既存コード・規約から一意に決まる記述を、根拠となるコードまたは規約を示してFindingにする。粒度の不足と超過を同じ重みで扱う。
- Requirements／Behaviorにない外部挙動、要求外の基盤が追加されていないか確認する。
- 根本原因に応じて`REQUIREMENTS`、`BEHAVIOR`、`DESIGN`のownerを付ける。

Findingがなければ`CLEAR`、あれば`FINDINGS`にする。正本の矛盾や必須資料不足で自動判断不能な場合だけ、一つの質問を付けて`NEEDS_HUMAN`にする。`target: DESIGN`と現在digestを提出し、文書を変更しない。
