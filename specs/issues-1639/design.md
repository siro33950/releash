# Design

## The actual design

### Architecture

#### 責務 owner と変更対象

本変更は Rust backend の `LocalEventStore` が所有する。schema shape と version evolution は既存どおり `src-tauri/src/adaptor/gateway/local_event_store/schema.rs`、writer lock 取得から reader pool 構築までの起動順序は `src-tauri/src/adaptor/gateway/local_event_store/store.rs` を正本とする。SQLite の物理回収と固定 database path の差し替えは、新設する `src-tauri/src/adaptor/gateway/local_event_store/maintenance.rs` に閉じ込め、`store.rs` は起動順序への組み込みだけを担う。

この owner は、Issue #1639 の確定方針、既存の単一 writer／固定 SQLite authority、[src-tauri/AGENTS.md](../../src-tauri/AGENTS.md)、[Gateway 規約](../../docs/architecture/GATEWAY.md)、[Infrastructure 規約](../../docs/architecture/INFRASTRUCTURE.md) に基づく。既存 app data GC、usecase、controller、frontend は変更しない。

主要な変更対象は次のとおり。

| Path | 変更の要旨 |
| --- | --- |
| `src-tauri/src/adaptor/gateway/local_event_store/schema.rs` | schema v5、未使用 schema object の削除、supported schema からの移行、現行 schema の不在検証を所有する。 |
| `src-tauri/src/adaptor/gateway/local_event_store/store.rs` | schema evolution と既存の checkpoint／sync 後、reader connection と worker の生成前に起動時保守を呼び出す。 |
| `src-tauri/src/adaptor/gateway/local_event_store/maintenance.rs` | 発火判定、`VACUUM INTO`、出力検証、固定 path の差し替え、失敗時 cleanup を所有する。 |
| `src-tauri/src/adaptor/gateway/local_event_store/layout.rs` | 保守用一時 database と、その SQLite sidecar の固定 path 導出を追加する。app data の列挙は行わない。 |
| `src-tauri/src/adaptor/gateway/local_event_store/fault.rs` | 物理回収失敗と差し替え境界を in-process で検証する fault point を追加する。production では従来どおり no-op とする。 |

schema evolution は store の正当性を確立する必須処理であり、失敗時は既存の `SchemaEvolutionFailed` で起動を止める。物理回収は正当性確立後の保守であり、失敗しても canonical database を変更せず、利用可能な writer connection を返して起動を続ける。この境界により R-001／R-002 と R-005 の失敗意味を混在させない。

| 上流契約 | 対応する設計判断 |
| --- | --- |
| R-001、R-002／B-001、B-002、B-003 | schema v5 の原子的移行と現行 schema の不在 invariant |
| R-003、R-004／B-004、B-005 | header-backed PRAGMA による AND 判定と起動時保守 |
| R-005、R-006／B-006、B-007 | 元 database を変更しない出力生成、検証後の原子的差し替え、sidecar 分離 |

#### 起動順序と原子性境界

`LocalEventStore::open` は writer lock file の排他保持中に、既存 database の分類、writer connection の open、必要な schema evolution、現行 schema 検証、metadata 更新、`wal_checkpoint(TRUNCATE)`、database sync を完了する。その後に起動時保守を実行し、保守が返した canonical writer connection から metadata を読み、reader pool と writer／reader worker を生成する。

物理差し替えの原子性境界は、同一 app data directory 内の保守用一時 database から固定 database path への platform 対応 replace とする。replace 前は元 database だけが authority、replace 成功後は検証・sync 済みの縮小 database だけが authority であり、pointer file、generation directory、backup authority は追加しない。writer lock は判定から reader pool 構築まで継続保持する。

#### 必要な検証

- B-001〜B-003 は、supported schema v1〜v4 の file-backed SQLite と新規 store の schema object を検査し、移行後と再起動後に対象 3 テーブル／index が存在しないことを検証する。
- B-004／B-005 は、発火判定を純粋な整数計算として境界値検証し、file-backed SQLite では発火時だけ固定 database の物理サイズと file identity が更新されることを検証する。非発火経路は保守用 fault point が到達しないことにより、`VACUUM INTO` が実行されないことを確認する。
- B-006 は `VACUUM INTO` と replace の前後へ fault を注入し、元 database の identity／内容で再 open でき、一時 database とその sidecar が残らないことを検証する。
- B-007 は縮小前後の使用中テーブルと `store_metadata` の内容、現行 schema 検証、再 open 後の read／write、および旧 `-wal`／`-shm` が差し替え後の authority に再利用されないことを file-backed SQLite で検証する。

検証コードは [Test 規約](../../docs/architecture/TEST.md) に従い対象 module 内へ置き、外部 process は起動しない。

### Interface

公開 command、Tauri／WebSocket protocol、domain trait は変更しない。既存の `LocalEventStore::open(LocalEventStoreConfig)` が唯一の入口であり、成功時は schema v5 かつ保守判定済みの store を返す。

内部境界として `maintenance.rs` の起動時保守は `StoreLayout` と唯一の writer connection の ownership を受け取り、元の connection または差し替え後に再 open した connection を返す。新しい trait や runtime command は追加しない。

schema evolution の公開失敗分類は既存の `SchemaEvolutionFailed` を維持する。物理回収の失敗分類は外部 interface に追加せず、保守内部で元 store への継続に収束させる。これにより既存 caller は物理回収の有無を条件にせず、従来と同じ open contract を利用できる。

### Data Model

永続 record は追加しない。`store_metadata.schema_version` と `PRAGMA user_version` を 5 に更新するが、`installation_id`、HMAC key、`process_instance_id`、各使用中 record の identity と内容は移行・縮小を通じて保持する。

保守用一時 database は `local-event-store.vacuum.sqlite3` という固定名で canonical database と同じ directory に置く。一時 database とその `-wal`／`-shm` は authority、履歴、再開 token として保持せず、起動時保守の入口と失敗出口で cleanup 対象にする。保守実行履歴、最終実行時刻、freelist の snapshot は永続化せず、発火条件は毎起動時の SQLite header 値から導出する。

### Database

schema v5 では `message_projection`、`terminal_records`、`stop_resolutions` と `idx_message_projection_ordinal` を現行 DDL から除去する。v4→v5 を追加し、既存の v1／v2／v3 移行も同じ最終 v5 transaction に収束させる。各経路は対象 object の削除、`store_metadata.schema_version = 5`、`PRAGMA user_version = 5` を同じ schema evolution transaction で確定し、commit 後に現行 schema を再検証する。

既存 database の分類は v4 を supported schema として受け入れる。`validate_current_schema` は対象 3 テーブルと index の不在を必須 invariant とし、index の存在要求から `idx_message_projection_ordinal` を外す。v1 入力の形を確認する `validate_supported_schema_v1` にある `terminal_records` の検証は、移行前 schema の識別根拠なので維持する。

物理回収の access path は、writer connection に対する `PRAGMA page_count`、`PRAGMA freelist_count`、`PRAGMA page_size` と `VACUUM INTO` に限定する。`page_count` と `freelist_count` は [SQLite PRAGMA](https://www.sqlite.org/pragma.html) が提供する page 数を使用し、テーブル走査や `dbstat` を発火判定へ使用しない。

### UI/UX

該当なし。

### Algorithm

#### 発火判定

page 数と page size は非負整数へ検証してから計算する。`page_count == 0` は非発火とし、それ以外では次の二条件をともに満たした場合だけ縮小する。

- `freelist_count * 4 >= page_count`
- `freelist_count * page_size >= 64 MiB`

浮動小数点除算を使わず cross multiplication と checked integer arithmetic を使うことで、25% 境界と 64 MiB 境界を丸めずに判定する。どちらか一方でも満たさない場合は同じ writer connection をそのまま返し、`VACUUM INTO`、一時 database の作成、database file の差し替えを行わない。

#### 物理回収と差し替え

保守の入口で前回中断に由来する固定一時 database とその sidecar を cleanup する。発火時は元 writer connection の `synchronous=FULL` を維持したまま `VACUUM INTO` で一時 database を生成する。[SQLite の VACUUM 仕様](https://www.sqlite.org/lang_vacuum.html) が保証する元 database と同じ logical content の consistent snapshot を利用し、完了後に一時 database の現行 schema／integrity、owner-only permission、file sync を確認する。検証が終わるまで元 database は変更しない。

一時 database の準備成功後に元 writer connection を閉じる。事前に完了した `wal_checkpoint(TRUNCATE)` と正常 close により committed WAL を本体へ反映した状態で、旧 canonical path の `-wal`／`-shm` を除去する。これは WAL が database の persistent state の一部であるという [SQLite WAL 仕様](https://www.sqlite.org/wal.html) に従い、open 中または checkpoint 前の WAL を単独で削除しないための順序制約である。

同一 directory 内で一時 database を canonical path へ原子的に replace し、directory を sync する。Unix では同一 filesystem の rename、Windows では repository で採用済みの `ReplaceFileW`／`MoveFileExW` write-through 方式を用いる。replace した database は既存の `open_existing_writer` から再 open し、そこで `journal_mode=WAL` と `synchronous=FULL` を再確立する。したがって `VACUUM INTO` 出力の journal mode を canonical mode と仮定せず、旧名の sidecar を新しい本体へ組み合わせない。

`VACUUM INTO`、一時 database の検証・sync・permission 設定、sidecar cleanup、replace のいずれかが replace 成功前に失敗した場合は、一時 artifact を cleanup し、canonical path の元 database をそのまま使用する。元 connection を閉じた後の失敗では、変更されていない canonical path を `open_existing_writer` で再 open する。失敗は correlation を付けた warning として記録するが、schema validation 成功済みの元 store の起動は継続する。

### Infra

新しい service、daemon、background task、dependency、deployment 設定は追加しない。保守は `LocalEventStore::open` を同期でブロックし、writer／reader worker が起動した後には実行しない。既存 app data GC の構成と実行時刻は変更しない。

## Alternatives Considered

- schema v5 の migration 内だけで物理回収する案は採用しない。schema 移行後に空き領域が再び閾値へ達した場合に回収経路がなくなり、R-003 を継続して満たせないためである。
- 既存 app data GC から回収する案は採用しない。GC 実行時には reader pool と writer worker が既に動作しており、安全な固定 path 差し替え境界を所有しないためである。
- canonical database に通常の `VACUUM` を直接実行する案は採用しない。失敗時にも元 file を不変で保持する R-005 の境界を、出力検証後の原子的差し替えとして構成できないためである。
- auto-vacuum を有効化する案は採用しない。既存 store の常設回収方式と schema／file 再構築の扱いを変更し、確定した `VACUUM INTO` と閾値発火の契約から外れるためである。

## Cross-cutting concerns

- 耐久性: 一時 database file、atomic replace、app data directory の順に sync し、replace 前の中断では元 database、replace 後の中断では検証済み database のどちらかだけを canonical path とする。
- 性能: 非発火起動では header-backed PRAGMA と整数計算だけを追加する。発火時の全 database 再構築は reader／worker 起動前に一度だけ同期実行する。
- セキュリティ: 一時 database にも canonical database と同じ owner-only permission を設定し、削除済み page の内容を一時 artifact として残さない。
- 可観測性: skip、成功、失敗を既存 logging 経路へ記録し、database 内容はログへ含めない。物理回収の成否を新しい public state や永続 record にはしない。

## Risks

該当なし。
