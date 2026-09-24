# Design 02

## 開始状態

差分の基準は `main` の `42be41e0`（`refactor(error): 失敗の分類をエラーの値に持たせる (#1880) (#1919)`）で、`feat/issues/1881` はこの commit から派生している。直前の Design は `docs/specs/issues-1881/design-01.md` であり、その「変える部分」は作業ツリーに未コミットの変更として実装済みである。読み込みの入口は `ReaderPool::submit`（async）だけになり、`ReaderPool::submit_blocking`、`LocalEventStore::submit_indexed_query_blocking`、`LocalEventReadStore::submit_indexed_query_blocking`、`store::sqlite_error_is_storage_unavailable` はいずれも残っていない。

この周までに解消・見送りとなった Thread は無い。`[REJECTED]` と判断した Thread、`[DEFERRED]` とした Thread はいずれも存在しない。

`requirements.md` と `behavior.md` は今周で変更していない。R-001〜R-006 と B-001〜B-009 の対応は対応表のとおり保たれており、誤り・不足・矛盾は見つからなかった。

## 変える部分

- 読み込みクロージャ内の非 SQLite な失敗を `DATA_LOSS` の分類にする: `reader.rs` の `canonical_runtime_owner_snapshot` にある 3 箇所（`records_from_tree_rows` の decode 失敗、`fold_execution_tree` の fold 失敗、fold 結果欠損）が返す `LocalEventQueryError::InvalidRequest`（`FailureKind::InvalidInput`）を `FailureKind::Corrupt` の失敗へ改める。根拠: Thread `73f38bb8-22a5-4c0d-bc2e-ab8170ca072c`（`[FIX_POLICY]`）。同じ decode／fold 失敗を `fact_log` の経路は `FactReadError::Corrupt` として扱っており、R-005「保存データの破損による失敗は `DATA_LOSS`」、B-009、Outcome「同じ失敗は、どの経路を通っても同じ分類で観測される」に反する。ルート: 固定（「規則の所有（読み込みクロージャ内の非 SQLite な失敗の分類）」。置き換えの手順は委任）
- design-01 の固定ルートのうち、`reader.rs` の 3 箇所への適用を解除する: design-01「規則の所有（`rusqlite::Error` の分類）」は置き換え対象 24 か所に `reader.rs` 3 か所を数えていたが、この 3 か所は `rusqlite::Error` ではなく `String` 由来の decode／fold 失敗であり、`&rusqlite::Error` を取る `reader::storage_unavailable` は適用できない。この 3 か所を同ルートの対象から外す。残る 21 か所（`execution_archive_repository.rs` 8、`fact_log.rs` 6、`startup_repository.rs` 4、`event_repository.rs` 2、`workflow_host.rs` 1）への指定は維持する。根拠: Thread `73f38bb8-22a5-4c0d-bc2e-ab8170ca072c` が示した事実（基準 HEAD `42be41e0` の `reader.rs:281`・`:311`・`:312` はいずれも非 SQLite な失敗）。ルート: 固定（「規則の所有（`rusqlite::Error` の分類）」の対象縮小）
- `WorkflowEventLogRepository` の read／read_page に失敗分類の回帰確認を足す: `event_repository.rs` の既存 3 テストは正常系だけで、読み込み失敗の分類が観測コードまで保たれることを確認するテストが無い。根拠: Thread `923cb82d-1cce-4e14-9617-36fed65a6bba`（`[FIX_POLICY]`）。`docs/architecture/TEST.md` は `adaptor/gateway/` のテストを必須とし、守る対象は R-004・R-005 と B-004・B-005・B-006・B-007・B-009。ルート: 委任
- `WorkflowRuntimeHost::load_execution_revision` の read 経路に失敗分類の回帰確認を足す: `workflow_host_test.rs` で `WorkflowRuntimeError::StorageFailure` を検査する既存テストは control-plane commit readback（write 側）だけで、この read 経路の分類確認が無い。根拠: Thread `4735149a-7ce0-4e0d-80af-860c31db3247`（`[FIX_POLICY]`）。守る対象は R-004・R-005 と B-004・B-005・B-006・B-007・B-009。ルート: 委任

## 固定するルート

- 規則の所有（読み込みクロージャ内の非 SQLite な失敗の分類）: `reader.rs` の既存の `corrupt` が所有する。これは `LocalEventQueryError::Corrupt`（`FailureKind::Corrupt` → `DATA_LOSS`）を返し、同ファイル内で stored event id / stream head / session projection の decode 失敗に既に使われている。`canonical_runtime_owner_snapshot` の 3 箇所（decode 失敗、fold 失敗、fold 結果欠損）をこの経路へ寄せる。範囲は `reader.rs` の読み込みクロージャのうち、`rusqlite::Error` ではない保存データ由来の失敗。`reader.rs` の引数検証（`limit` の範囲検査 2 か所）は対象外。粒度は所有者と分類先（`FailureKind::Corrupt`）までの指定で、置き換えの手順と関数の可視性は委任。理由は、同じ decode／fold 失敗が `fact_log` 経由では `FactReadError::Corrupt` として `DATA_LOSS` になり、経路によって分類が割れているため。関係する要求は R-005 と B-009。
- 規則の所有（`rusqlite::Error` の分類）: design-01 で固定したものを維持する。`reader::sqlite_failure_kind` が所有し、読み込み経路の `map_err(|_| LocalEventQueryError::InvalidRequest)` を `reader::storage_unavailable` 経由へ置き換える。ただし対象から `reader.rs` の 3 か所を外し、21 か所とする。
- 規則の所有（SQLite エラー分類の一本化先と分類表）: design-01 で固定したものを維持する。
- 規則の所有（fact log の読み込みの失敗の型）: design-01 で固定したものを維持する。`FactReadError` が所有する。
- 読み込みの入口: design-01 で固定したものを維持する。`ReaderPool::submit` を唯一の入口とし、`tokio-rusqlite` クレートは導入しない。
- async 化の対象範囲: design-01 で固定したものを維持する。

## 変えないもの

- `reader.rs` の引数検証による `LocalEventQueryError::InvalidRequest`（`limit` の範囲検査 2 か所）。これは本来の用途であり、`INVALID_ARGUMENT` のまま維持する。Thread `73f38bb8-22a5-4c0d-bc2e-ab8170ca072c` の受入条件が対象外と定めているため。

## 未確定・リスク

- 自動判断（design-01 の周）: R-005 が挙げる 3 つの観測のうち、保存データの破損による失敗が `DATA_LOSS` になることに対応する受入条件が無かったため、B-009 を追加し、対応表の R-005 の行を更新した。`requirements.md` の Assumptions に記録が残っている。今周で新たに自動判断した箇所は無い。
- 未決のまま残した要求は無い。`[DEFERRED]` で人間へ渡した件も無い。
