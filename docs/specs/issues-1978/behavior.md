## B-001: 汎用の購読の処理は terminal を区別しない

汎用の購読の処理（購読の Usecase、その配信の口、汎用の presenter、配信の土台）は、対象が terminal かどうかによる分岐、terminal 専用の状態、terminal 専用の手続き、terminal 専用の引数を持たない。

## B-002: terminal の購読で今の状態と変化が届く

GIVEN 動いている terminal がある
WHEN 画面がその terminal の購読を開始する
THEN 購読の stream に、その terminal の今の状態（再現する画面、その寸法、終了の有無と終了コード、表示名）が届く
AND 以後、その terminal の出力、寸法の変更、終了が差分として届く

## B-003: terminal 以外の購読で今の状態と変化が届く

GIVEN terminal 以外の購読対象がある
WHEN 画面がその対象の購読を開始する
THEN 購読の stream に、その対象の今の状態が届く
AND 以後、その対象が変わるたびに変わった状態が届く

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
