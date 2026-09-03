## B-001: プレーンテキスト URL のリンク認識

GIVEN terminal にプレーンテキストの `http` / `https` URL が表示されている
WHEN 利用者がその URL 上へポインタを合わせる
THEN その URL がクリック可能なリンクとして提示される

## B-002: プレーンテキスト URL のクリックによる外部ブラウザ起動

GIVEN terminal にプレーンテキストの `http` / `https` URL が表示されている
WHEN 利用者がその URL をクリックする
THEN OS の既定ブラウザが当該 URL を開く
AND Releash 本体の WebView は当該 URL へ遷移しない

## B-003: OSC 8 ハイパーリンクのクリックによる外部ブラウザ起動

GIVEN terminal に遷移先が `http` / `https` の OSC 8 ハイパーリンクが表示されている
WHEN 利用者がそのリンクをクリックする
THEN OS の既定ブラウザが当該 URL を開く
AND Releash 本体の WebView は当該 URL へ遷移しない

## B-004: hover による遷移先の提示

GIVEN terminal にリンク（プレーンテキスト由来または OSC 8 由来）が表示されている
WHEN 利用者がそのリンクへポインタを合わせる
THEN 遷移先の URL 全体が提示される
AND 利用者がそのリンクからポインタを外すと、その提示は消える

## B-005: クリックが確認の応答を求めない

GIVEN terminal にリンク（プレーンテキスト由来または OSC 8 由来）が表示されている
WHEN 利用者がそのリンクをクリックする
THEN 確認の応答を求められることなく、OS の既定ブラウザが当該 URL を開く

## B-006: 折り返しで分断された URL の扱い

GIVEN terminal の幅を超える URL が折り返され、複数行に分断されて表示されている
AND 分断された URL 全体が、利用者がクリックする行から上方向・下方向それぞれ 2,048 文字までの連結範囲に収まっている
WHEN 利用者が分断されたいずれかの行の URL 部分をクリックする
THEN OS の既定ブラウザが、分断前の URL 全体を開く

## B-007: `http` / `https` 以外のスキームはリンクにならない

GIVEN terminal に遷移先のスキームが `http` / `https` でない文字列または OSC 8 リンクが表示されている
WHEN 利用者がその箇所へポインタを合わせ、クリックする
THEN その箇所はクリック可能なリンクとして提示されない
AND OS の既定ブラウザは開かない

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002, B-006 |
| R-002 | B-003 |
| R-003 | B-002, B-003 |
| R-004 | B-007 |
| R-005 | B-004 |
| R-006 | B-005 |
| R-007 | B-006 |
