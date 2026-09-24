## B-001: 状態が購読で届く

GIVEN daemon が持つ状態が購読の対象になっている
WHEN client がその対象を購読する
THEN その状態が購読で届く
AND その状態を返す単発の呼び出しは無い

## B-002: 操作と計算は単発の呼び出しで行える

GIVEN client が daemon へ接続している
WHEN 状態を変える操作、または client の入力に対する計算を要求する
THEN 単発の呼び出しとして結果が返る

## B-003: 購読の単位は読み取り結果 1 つ

GIVEN 画面が一つの読み取り結果を使う
WHEN client がその対象を購読する
THEN 組み合わせ済みの読み取り結果が 1 つ届く
AND client は届いた結果をそのまま表示に使える

## B-004: 最初の状態と区切りの印

GIVEN 存在する対象がある
WHEN client がその対象を購読する
THEN まず現在の状態が届く
AND 続いて区切りの印が届く
AND 区切りの印より後は、その対象の変更だけが届く

## B-005: 変更は対象を丸ごと送る

GIVEN 本 ISSUE で購読へ移した対象を購読している
WHEN その対象が変わる
THEN 変わった後の対象が丸ごと届く

## B-006: 全ての購読が 1 本の stream で届く

GIVEN 一つの client が複数の対象を購読している
WHEN それぞれの対象が変わる
THEN 全ての変更が、その client の 1 本の stream で届く

## B-007: 購読の開始と停止

GIVEN client が stream を持っている
WHEN 購読の開始または停止を単発の呼び出しで要求する
THEN その要求が受け付けられる
AND 停止した対象の変更は、以後その stream に届かない

## B-008: 版番号が付く

GIVEN client が対象を購読している
WHEN 状態、変更、印のいずれかが届く
THEN それぞれに、その対象の版番号が付いている
AND 同じ daemon の起動の中では、後に届いたものの版番号は、先に届いたものの版番号より小さくならない

## B-009: 最後に受け取った版からの再開

GIVEN client が対象の版番号を受け取った後に stream が切れた
WHEN 最後に受け取った版を指定して購読し直す
THEN その版より後の変更が届く

## B-010: 再開できない版を指定したときのやり直し

GIVEN client が指定する版から再開できない（daemon が再起動した後に、再起動前の版を指定した場合を含む）
WHEN その版を指定して購読し直す
THEN 現在の状態が最初から届く
AND 続いて区切りの印が届く

## B-011: 送り待ちが溢れた購読だけが再開する

GIVEN 一つの client が複数の対象を購読している
WHEN ある購読の送り待ちが溢れる
THEN その購読は版からの再開になる
AND 同じ stream の他の購読への配信は続く
AND stream は終わらない

## B-012: 変更が無い間の印

GIVEN client が対象を購読していて、その対象が変わっていない
WHEN 印の間隔が過ぎる
THEN 版番号を載せた印が届く

## B-013: 購読の数に上限が無い

GIVEN client が既に対象を購読している
WHEN さらに別の対象を購読する
THEN 購読の数を理由に拒否されない

## B-014: 同じ対象の重複購読は 1 件にまとまる

GIVEN client が一つの stream で対象を購読している
WHEN 同じ stream で同じ対象を再び購読する
THEN その対象の購読は 1 件のままである
AND その対象の変更は 1 回だけ届く

## B-015: 存在しない対象の購読は拒否される

GIVEN 存在しない対象を指定する
WHEN 購読の開始を要求する
THEN その要求は拒否される

## B-016: stream の終了で購読が全て終わる

GIVEN client が複数の対象を購読している
WHEN その client の stream が終わる
THEN その client の購読は全て終わる

## B-017: Repository のパス一覧が購読で届く

GIVEN client が Repository のパス一覧を購読する
WHEN 購読を開始する
THEN 現在の Repository のパス一覧が届く
AND 以後、Repository のパス一覧が変わるたびに、変わった後の一覧が届く

## B-018: Repository のパス一覧を返す単発の呼び出しの不在

daemon は、Repository のパス一覧を返す単発の呼び出しを提供しない。

## B-019: Repository のパス一覧の変更通知の不在

daemon は、Repository のパス一覧が変わったことを知らせる変更通知を送らない。

## B-020: Repository の追加・削除後の Workspaces の表示

GIVEN Workspaces が Repository 一覧を表示している
WHEN Repository を追加する、または削除する
THEN Workspaces が表示する Repository 一覧に、その追加または削除が反映される

## B-021: 使われていない読み取りの呼び出しの不在

daemon は、R-020 が挙げる読み取りの呼び出しを提供しない。

## B-022: 使われなくなるコードの不在

この変更で使われなくなったコードは残らない。

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-006 |
| R-007 | B-007 |
| R-008 | B-008 |
| R-009 | B-009, B-010 |
| R-010 | B-011 |
| R-011 | B-012 |
| R-012 | B-013 |
| R-013 | B-014 |
| R-014 | B-015 |
| R-015 | B-016 |
| R-016 | B-017 |
| R-017 | B-018 |
| R-018 | B-019 |
| R-019 | B-020 |
| R-020 | B-021 |
| R-021 | B-022 |
