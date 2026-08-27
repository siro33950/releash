## B-001: 接続先を確認できなかった場合の失敗表示

GIVEN discovery file が存在し、その内容の検査を通過する
AND 実行環境が discovery の接続先への接続を許可しない
WHEN local API を使う CLI command を実行する
THEN 失敗表示は、接続先を確認できなかったこと（接続不能または環境による遮断）を失敗の理由として示す
AND 失敗表示は、discovery file が不正または古いことを失敗の理由として示さない

## B-002: 接続先が別インスタンスを指す場合の失敗表示

GIVEN discovery file が存在し、その内容の検査を通過する
AND discovery の接続先へ到達できる
AND その接続先が discovery の instance_id を持つ local API ではない
WHEN local API を使う CLI command を実行する
THEN 失敗表示は、discovery が別のインスタンスを指すか陳腐化していることを失敗の理由として示す
AND 失敗表示は、接続先を確認できなかったことを失敗の理由として示さない

## B-003: discovery が指すプロセスが存在しない場合の接続拒否

GIVEN 実行環境が discovery の pid に対するプロセス情報を参照できる
AND discovery が指す pid のプロセスが存在しない
WHEN local API を使う CLI command を実行する
THEN その discovery を用いた local API への接続は拒否される
AND command は失敗する

## B-004: discovery の開始時刻が一致しない場合の接続拒否

GIVEN 実行環境が discovery の pid に対するプロセス情報を参照できる
AND discovery が指す pid のプロセスの開始時刻が discovery の process_started_at と一致しない
WHEN local API を使う CLI command を実行する
THEN その discovery を用いた local API への接続は拒否される
AND command は失敗する

## B-005: 接続先の確認が成立しない場合の token 非送信

GIVEN discovery file が存在し、その内容の検査を通過する
AND 接続先が discovery の instance_id を持つ local API であることを確認できない
WHEN local API を使う CLI command を実行する
THEN 接続先へ discovery の token を含む request は送信されない
AND command は失敗する

## B-006: local API が起動していない場合の失敗表示と終了コード

GIVEN discovery file が存在しない
WHEN `releash workflow output submit` を実行する
THEN stderr へ `error: この操作には Releash アプリの起動が必要です` が出力される
AND 終了コードは 1 である

## B-007: プロセス情報を参照できない場合の専用失敗

GIVEN discovery file が存在し、その内容の検査を通過する
AND 実行環境からプロセス情報を参照できない
WHEN local API を使う CLI command を実行する
THEN discovery は不正・陳腐化とは区別した専用の失敗として拒否される
AND 失敗表示はプロセス情報を参照できないため接続先を確認できなかったことを示す
AND 失敗表示は discovery file の path、token、「不正」「古い」の語を含まない
AND identity GET を含む接続先への request は送信されない
AND command は file fallback へ進まず終了コード 1 で失敗する

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-003, B-004 |
| R-003 | B-005 |
| R-004 | B-006 |
| R-005 | B-007 |
