## B-001: 旧schemaストアから未使用データを除去する

GIVEN 対応対象の旧schemaバージョンのストアに`message_projection`、`terminal_records`、`stop_resolutions`、`idx_message_projection_ordinal`、および残存データが存在する
WHEN 現行版がそのストアを開く
THEN ストアは移行後のschemaで利用可能になる
AND 3テーブル、`idx_message_projection_ordinal`、および残存データはストアに存在しない

## B-002: 新規ストアに未使用テーブルを作成しない

GIVEN local event storeがまだ存在しない
WHEN 現行版がストアを新規作成する
THEN 作成されたストアに`message_projection`、`terminal_records`、`stop_resolutions`、および`idx_message_projection_ordinal`は存在しない

## B-003: 未使用テーブルを再作成しない

GIVEN 移行または新規作成によって現行schemaになったストアが存在する
WHEN 現行版がそのストアを再び開く
THEN `message_projection`、`terminal_records`、`stop_resolutions`、および`idx_message_projection_ordinal`は再作成されない

## B-004: 発火条件を満たす空き領域を起動時に回収する

GIVEN ストアの`freelist_count / page_count`が25%以上であり、かつfreelistが64MB相当以上である
WHEN 現行版がそのストアを開く
THEN 起動時に空き領域がストア本体ファイルの物理サイズから回収される
AND ストアは回収後のファイルで利用可能になる

## B-005: 発火条件を満たさない起動では物理縮小を行わない

GIVEN ストアの`freelist_count / page_count`が25%未満、またはfreelistが64MB相当未満である
WHEN 現行版がそのストアを開く
THEN ストア本体ファイルの物理縮小は実行されない
AND 発火判定のためのデータベースファイル走査は発生しない

## B-006: 物理縮小の失敗後も元のストアで起動する

GIVEN 発火条件を満たすストアの物理縮小がディスク不足等により完了できない
WHEN 現行版がそのストアを開く
THEN 物理縮小用の一時ファイルは残らない
AND 元のストアファイルのまま起動が継続し、ストアを利用できる

## B-007: 物理縮小後も使用中データの整合性を維持する

GIVEN 発火条件を満たすストアの使用中テーブルと`store_metadata`にデータが存在する
WHEN 現行版が物理縮小とストア本体ファイルの差し替えを完了する
THEN 使用中テーブルと`store_metadata`のデータは縮小前と同じ内容で利用できる
AND 差し替え前の`-wal`または`-shm`ファイルに由来する不整合は観測されない

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002, B-003 |
| R-003 | B-004 |
| R-004 | B-005 |
| R-005 | B-006 |
| R-006 | B-007 |
