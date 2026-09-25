## B-001: terminal が同じ購読の stream で届く

GIVEN client が terminal と他の対象を購読している
WHEN terminal の状態と他の対象の状態がそれぞれ変わる
THEN 全ての変化が、その client の 1 本の購読の stream で届く

## B-002: terminal の変化が差分として届く

GIVEN client が terminal を購読している
WHEN terminal が変わる
THEN 変わった分だけが差分として届く
AND UI は受け取った差分を画面へ反映する

## B-003: terminal の最初の状態

GIVEN 動いている terminal がある
WHEN client がその terminal を購読する
THEN 現在の状態として、再現する画面、その寸法、終了の有無、終了している場合の終了コードが届く

## B-004: terminal の以後の変化

GIVEN client が terminal を購読していて、最初の状態を受け取っている
WHEN 出力、寸法の変更、終了のいずれかが起きる
THEN その変化が届く

## B-005: terminal の版番号は出力の番号

GIVEN client が terminal を購読している
WHEN 状態、変化、印のいずれかが届く
THEN それぞれに、その terminal の出力の番号が版番号として付いている

## B-006: 最後に受け取った版番号からの再開

GIVEN client が terminal の版番号を受け取った後に stream が切れた
WHEN 最後に受け取った版番号を指定して購読し直す
THEN その版番号より後の変化が届く

## B-007: 再開できない版番号を指定したときのやり直し

GIVEN client が指定する版番号から再開できない
WHEN その版番号を指定して terminal を購読し直す
THEN 現在の状態が最初から届く

## B-008: terminal 専用の購読の stream の不在

daemon は、terminal 専用の購読の stream を提供しない。

## B-009: 出力のひとかたまりごとの受信確認の不在

daemon は、出力のひとかたまりごとの受信確認を受け付けない。

## B-010: 処理済みの量のまとめた通知

GIVEN UI が terminal の購読の最初の状態として通知単位を受け取り、その terminal の出力を受け取って処理している
WHEN 処理し終えた量が、受け取った通知単位に達する
THEN UI はその量を daemon へ知らせる

## B-011: 上限を超えたときの出力元の一時停止

GIVEN daemon が terminal の出力を配信している
WHEN 知らせを受け取っていない量が上限を超える
THEN daemon はその terminal の出力元を一時停止する

## B-012: 下限を下回ったときの出力元の再開

GIVEN terminal の出力元が一時停止している
WHEN 処理済みの量の知らせにより、知らせを受け取っていない量が下限を下回る
THEN daemon はその terminal の出力元を再開する

## B-013: 送る量の制御の値

送る量の制御の量の単位は UTF-16 code unit であり、上限は 100K、下限は 5K、通知単位は 5K である。通知単位は下限以下である。

## B-014: 最初の状態の作成が他の terminal の出力を止めない

GIVEN 複数の terminal が動いていて、出力している
WHEN client がそのうち 1 つの terminal を購読し、その最初の状態が作られている
THEN 他の terminal の出力は届き続ける

## B-015: 最初の状態の作成が他の呼び出しを止めない

GIVEN terminal が動いている
WHEN client がその terminal を購読し、その最初の状態が作られている
THEN daemon は他の呼び出しに応答する

## B-016: 複数の terminal の最初の状態を同時に作れる

GIVEN 複数の terminal が動いている
WHEN client がそれらを同時に購読する
THEN それぞれの最初の状態は、互いの完了を待たずに作られる

## B-017: 一時停止が他の terminal の出力を止めない

GIVEN 複数の terminal が動いていて、出力している
WHEN そのうち 1 つの terminal の出力元が送る量の制御で一時停止する
THEN 他の terminal の出力は届き続ける

## B-018: 一時停止が他の呼び出しを止めない

GIVEN terminal の出力元が送る量の制御で一時停止している
WHEN client が他の呼び出しを行う
THEN daemon はその呼び出しに応答する

## B-019: 一時停止中も寸法の変更を受け付ける

GIVEN terminal の出力元が送る量の制御で一時停止している
WHEN その terminal の寸法の変更を要求する
THEN その要求は受け付けられる

## B-020: terminal の状態を返す読み取りの呼び出しの不在

daemon は、terminal の状態を返す単発の読み取りの呼び出しを提供しない。

## B-021: terminal を起動する操作

GIVEN client が terminal を起動しようとする
WHEN 起動の操作を要求する
THEN その terminal が使える状態になる
AND その操作の結果として、その terminal を指す識別子が返る
AND その操作の結果として terminal の状態は返らない

## B-022: terminal 専用の attach の不在

GIVEN client が stream を持っている
WHEN terminal の購読の開始または停止を要求する
THEN 他の購読対象と同じ手段で受け付けられる
AND daemon は terminal 専用の attach の呼び出しを提供しない

## B-023: terminal の数に terminal 固有の上限が無い

GIVEN client が既に複数の terminal を購読している
WHEN さらに別の terminal を購読する
THEN terminal の数を理由に拒否されない

## B-024: terminal への入力

GIVEN client が terminal を購読していて、その terminal が動いている
WHEN 利用者が terminal へ入力する
THEN その入力は terminal へ届く

## B-025: 使われなくなるコードの不在

この変更で使われなくなったコードは残らない。

## B-026: 複数の client が同じ terminal を購読しているときの判定

GIVEN 複数の client が同じ terminal を購読している
WHEN そのうち 1 つの client が出力を処理せず、その購読の知らせを受け取っていない量が上限を超える
THEN daemon はその terminal の出力元を一時停止する

## B-027: 通知単位が最初の状態として届く

GIVEN client が terminal を購読する
WHEN 最初の状態が届く
THEN 送る量の制御の通知単位が含まれる

## B-028: 入力が受け付けられなかったことの伝わり方

GIVEN client が terminal を購読していて、その terminal へ入力する
WHEN その入力が受け付けられない
THEN その入力の呼び出しの応答が失敗を伝える
AND terminal の購読では、入力が受け付けられなかったことは届かない

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003, B-004 |
| R-004 | B-005 |
| R-005 | B-006, B-007 |
| R-006 | B-008 |
| R-007 | B-009 |
| R-008 | B-010, B-027 |
| R-009 | B-011, B-012, B-026 |
| R-010 | B-013 |
| R-011 | B-014 |
| R-012 | B-015 |
| R-013 | B-016 |
| R-014 | B-017 |
| R-015 | B-018 |
| R-016 | B-019 |
| R-017 | B-020 |
| R-018 | B-021 |
| R-019 | B-022 |
| R-020 | B-023 |
| R-021 | B-024 |
| R-022 | B-025 |
| R-023 | B-028 |
