## B-001: 状態色の意味

GIVEN Workspace ツリーの行または詳細ペインのノード状態アイコンが表示される
WHEN ノードの4分類が色で表現される
THEN 青は「実行中」を表す
AND 黄は「介入が必要」を表す
AND 赤は「失敗」を表す
AND 緑は「動いていない」を表し、完了だけを意味しない

## B-002: Recovery fence を持つ leaf

GIVEN recovery fence を持つ leaf ノードが存在する
WHEN Workspace ツリーが表示される
THEN その leaf 行は赤で表示される
AND 詳細な状態が `paused` であっても赤で表示される

## B-003: 失敗した leaf

GIVEN recovery fence を持たず、詳細な状態が `failed` の leaf ノードが存在する
WHEN Workspace ツリーが表示される
THEN その leaf 行は赤で表示される

## B-004: Approval 待ちの leaf

GIVEN recovery fence を持たず、詳細な状態が `failed` ではない approval 待ちの leaf ノードが存在する
WHEN Workspace ツリーが表示される
THEN その leaf 行は黄で表示される

## B-005: Stop 信号だけを受領した実行中の leaf

GIVEN recovery fence を持たず、詳細な状態が `failed` でも approval 待ちでもない leaf ノードが存在する
AND その leaf ノードの詳細な状態は `running` である
AND その leaf ノードは Stop 信号を受領し、Submit 信号を受領していない
WHEN Workspace ツリーが表示される
THEN その leaf 行は黄で表示される

## B-006: 実行中の leaf

GIVEN recovery fence、`failed`、approval 待ち、実行中の Stop 信号だけの受領のいずれにも該当しない leaf ノードが存在する
AND その leaf ノードの詳細な状態は `running` である
WHEN Workspace ツリーが表示される
THEN その leaf 行は青で表示される

## B-007: 動いていない leaf

GIVEN より上位の分類条件に該当しない leaf ノードが存在する
AND その leaf ノードの詳細な状態は `paused`、`completed`、`aborted` のいずれかである
WHEN Workspace ツリーが表示される
THEN その leaf 行は緑で表示される
AND `paused` の leaf ノードが Stop 信号を受領していても緑で表示される

## B-008: 親行の重大度集約

GIVEN Sequence または Fanout の親行に、自分自身と配下の子の分類結果が存在する
WHEN Workspace ツリーが表示される
THEN 親行は赤、黄、青、緑の重大度順で最も重い分類の色で表示される
AND 自分自身と配下の子の分類結果が同じ組み合わせなら、Sequence と Fanout の親行は同じ色で表示される

## B-009: Workspace ツリー取得の状態表現

GIVEN Workspace ツリー取得の外部インターフェースを利用する
WHEN ツリー内のノードが返される
THEN 各ノードの状態表現は「実行中」「介入が必要」「失敗」「動いていない」の4分類のいずれかである
AND `running`、`paused`、`failed`、`waiting`、`aborted`、`completed` の詳細な状態値は返されない
AND 呼び出し側が分類を導出する必要はない

## B-010: Workspace ノード詳細取得の状態表現

GIVEN Workspace ノード詳細取得の外部インターフェースを利用する
WHEN ノード詳細が返される
THEN ノードの詳細な状態と4分類の両方が返される
AND 呼び出し側が詳細な状態から4分類を導出する必要はない

## B-011: 詳細ペインの状態アイコン

GIVEN Workspace ノードの詳細が表示される
WHEN 詳細ペインにノード状態アイコンが表示される
THEN アイコンの色はノードの4分類に従う
AND `paused`、`completed`、`aborted` を含む詳細な状態の違いはアイコン形状で判別できる

## B-012: interrupted の非公開化

GIVEN Workspace ツリー取得または Workspace ノード詳細取得の外部インターフェースを利用する
WHEN ノードの状態表現が返される
THEN `interrupted` はツリーの状態にもノード詳細の状態にも現れない

## B-013: ツリー行の状態アイコンの pulse

GIVEN Workspace ツリーの行にノード状態アイコンが表示される
WHEN ノードの4分類が青または黄である
THEN アイコンは pulse する
AND ノードの4分類が赤または緑である場合は pulse しない

## B-014: 操作可否と resume 不能理由の維持

GIVEN 同じ Workspace と workflow の状態が存在する
WHEN 状態分類の変更前後で利用可能な操作と resume 不能理由を比較する
THEN approve、retry、stop、resume、abort、archive の各操作可否は変わらない
AND resume 不能理由の内容は変わらない

## B-015: local API と CLI の応答維持

GIVEN 同じ要求を local API または CLI へ送る
WHEN 状態分類の変更前後で応答を比較する
THEN local API の応答は変わらない
AND CLI の応答は変わらない

## B-016: 詳細ペインの状態アイコンの pulse

GIVEN Workspace ノードの詳細が表示される
WHEN 詳細ペインにノード状態アイコンが表示される
THEN アイコンは4分類のいずれであっても pulse しない

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002, B-003, B-004, B-005, B-006, B-007 |
| R-003 | B-008 |
| R-004 | B-009 |
| R-005 | B-010 |
| R-006 | B-011 |
| R-007 | B-012 |
| R-008 | B-013, B-016 |
| R-009 | B-014 |
| R-010 | B-015 |
