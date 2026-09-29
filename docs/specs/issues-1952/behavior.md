## B-001: snapshot を待っている購読しか無い stream にも合図が届く

GIVEN client が購読の stream を開き、その stream の購読が全て snapshot の作成を待っている
WHEN 合図の間隔が経過する
THEN client はその stream で daemon からの合図を受け取る

## B-002: 購読を 1 つも持たない stream にも合図が届く

GIVEN client が購読の stream を開き、購読を 1 つも始めていない
WHEN 合図の間隔が経過する
THEN client はその stream で daemon からの合図を受け取る

## B-003: 合図が途絶えると生存の失敗を 1 回数える

GIVEN シェルが daemon との stream を開いている
WHEN daemon からの合図が 20 秒を超えて届かない
THEN シェルは生存の失敗を 1 回数え、その stream をつなぎ直す
AND daemon が「居ない」とは判定しない

## B-004: stream が切れると生存の失敗を 1 回数える

GIVEN シェルが daemon との stream を開いている
WHEN stream が切れる
THEN シェルは生存の失敗を 1 回数え、stream をつなぎ直す
AND daemon が「居ない」とは判定しない

## B-005: 購読の開始の失敗は生存の失敗として数えない

GIVEN シェルが daemon との stream を開き、desktop 設定の購読を始めようとしている
WHEN 購読の開始が失敗する
THEN シェルは生存の失敗を数えない
AND proto の規則がつなぎ直しの対象とする失敗であれば、シェルは stream をつなぎ直す

## B-006: 生存の失敗が連続 2 回に達すると「居ない」と判定する

GIVEN シェルが生存の失敗を 1 回数えている
WHEN 続けてもう 1 回、生存の失敗が起きる
THEN シェルは daemon が「居ない」と判定する

## B-007: 合図が届くと連続の数えが 0 に戻る

GIVEN シェルが生存の失敗を 1 回数えている
WHEN daemon からの合図が届く
THEN 連続の数えは 0 に戻る
AND その後 1 回だけ生存の失敗が起きても、daemon が「居ない」とは判定しない

## B-008: シェルは daemon との stream を 1 本だけ開く

GIVEN シェルが daemon に接続している
WHEN 画面が daemon の状態を読める状態になる
THEN シェルが daemon に開いている購読の stream は 1 本である
AND シェルはその stream で desktop 設定を受け取る

## B-009: シェルのつなぎ直しの待ちが proto の規則に従う

GIVEN シェルの stream が切れた
WHEN シェルがつなぎ直す
THEN つなぎ直しまでの待ちは、proto の規則で定めた初回の待ち・倍率・ずれ・上限に従う

## B-010: シェルの単発の呼び出しに proto の規則の既定の期限が当たる

GIVEN シェルが期限を指定せずに daemon へ単発の呼び出しを送る
WHEN daemon がその呼び出しに応答しない
THEN 呼び出しは proto の規則で定めた既定の期限で期限切れになる

## B-011: 画面とシェルが同じ無音の時間を使う

GIVEN 画面とシェルがそれぞれ daemon との stream を開いている
WHEN daemon からの合図が 20 秒を超えて届かない
THEN 画面もシェルも、その stream を切ってつなぎ直す

## B-012: シェルが観測した接続の失敗が分類付きで画面に届く

GIVEN シェルが daemon を「居ない」と判定した
WHEN 画面が daemon の状態を読む
THEN 失敗が、その分類が分かる形で示される

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-003, B-004, B-005 |
| R-003 | B-006, B-007 |
| R-004 | B-008 |
| R-005 | B-005, B-009 |
| R-006 | B-010 |
| R-007 | B-003, B-011 |
| R-008 | B-012 |
