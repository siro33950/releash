# Design 11

## 開始状態

- 差分の基準は base ブランチ `main`、派生点 `81ec380b`。branch `feat/issues/1840` には commit `c0c7c985`（Design 01〜06 の実装と spec 8 ファイル）と `8aa63a4e`（承認待ち Command を記録から承認待ちとして読み戻す）が積まれ、さらに Design 07・08・09・10 の実装が未コミットの作業ツリー差分（23 ファイル、+1168 / -380）として存在する。この 2 commit と作業ツリー差分をこの周の開始状態として扱う。
- 直前の Design は `docs/specs/issues-1840/design-10.md`。その「変える部分」1 項目は実装済みである。`restart_node_attempt`（`src-tauri/src/adaptor/gateway/workflow/workflow_host/node_startup.rs:211-294`）は `runtime_activation_gate` を取った内側を `retry_runtime_conflicts`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:255`）で包み、Conflict のとき最新の記録から再評価して有界に再試行する。`retry_failed_nodes`（`src-tauri/src/usecase/workflow/node_startup.rs:15-44`）は `gateway.restart` の Err を `first_error` へ退避して同一バッチの後続 ID の処理を継続し、ループ終了後にその Err を返す。対応するテストは `src-tauri/src/usecase/workflow/node_startup_test.rs` にある。
- この周までに解消・見送りとなった Thread: resolve 済み 20 件（Design 10 の対象 `cc4b65b0-860f-43ee-942c-41f4e79b2681` を含む）。`[REJECTED]`・`[DEFERRED]` とした Thread は無い。
- この周の対象は `[FIX_POLICY]` が付いた open Thread 1 件（`0ef529f3-6546-4335-837b-cb053f9f710c`）である。
- Requirements・Behavior はこの周で変更していない。R-001〜R-011 と B-001〜B-014、対応表は Design 10 の周と同じである。

## 変える部分

- 自動再起動バッチで restart が非 Conflict エラーで失敗したとき、失敗として記録される Node をそのエラーの発生元 Node に限る: `retry_failed_nodes`（`src-tauri/src/usecase/workflow/node_startup.rs:15-44`）は `gateway.restart` の非 Conflict Err を `first_error` へ退避して同一バッチの後続 ID を処理し、後続の `gateway.start` まで行った後にその Err を返す。呼び出し元の `schedule_startup_retries`（`src-tauri/src/adaptor/gateway/workflow/workflow_host/node_startup.rs:96-107`）は error と execution_id だけを `settle_runtime_failure` へ渡し、`settle_runtime_failure`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:1973-1997`）は発生元の Node ではなく `node_executions` を逆順に走査した末尾の active な非 composite Node を選ぶ。`restart_node_attempt_at`（`src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs:1865-1960`）は `push_started_node`（同 `:1175`）で新しい attempt を `node_executions` の末尾へ足すため、先行 Node の restart が失敗し後続 Node の restart が成功すると、その後続 Node の新しい attempt が末尾の active な非 composite Node となり、起動に成功した後続 Node が `NodeFailed` の対象になる。`restart_node_attempt`（`workflow_host/node_startup.rs:211-294`）は Conflict を `retry_runtime_conflicts` で吸収するが、presence 判定の `InvalidState`（同 `:240-245`）と記録の読取・commit の `SessionStore` を返し得るため、非 Conflict Err の経路は実在する。変更後は、先行 Node の restart が非 Conflict エラーで失敗し同一バッチの後続 Node の restart / start が成功した場合、失敗として記録される Node はそのエラーの発生元 Node だけであり、起動に成功した後続 Node の記録は変わらない。根拠: Thread `0ef529f3-6546-4335-837b-cb053f9f710c`。`requirements.md`「Scope / Non-goals」の「変更しない対象」にある「#1839 が入れた Node の起動失敗の自動再試行、プロセス在否の判定とその読み取り、Node の状態の値と導出規則」。`AGENTS.md`「構成で押さえる点」の「永続化は event store。事実を追記し、読み側で projection を導出する」に対し、起動に成功した Node へ `NodeFailed` を追記することは実際に起きていない事実の追記である。#1839 の自動再試行の回数と間隔の扱いは変えない。B-001〜B-014 の観測結果は変わらない。ルート: 委任

## 固定するルート

- この周に人間が新しく固定した実装上の指定は無い。上の 1 件はルートが委任である。非 Conflict エラーを発生元 Node と対応付ける方法（`retry_failed_nodes` が Err を `node_execution_id` とともに保持して呼び出し元へ渡すか、`schedule_startup_retries` が `settle_runtime_failure_for_node` を使う形にするか、その他）、および同一バッチの後続 Node の処理を継続したまま発生元 Node だけを精算する形は、いずれも実装側で決める。
- Design 01 で固定した D1〜D8 を維持する。

## 変えないもの

- `settle_runtime_failure_for_node`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:2001-2031`）が Conflict を対象 Node の失敗として記録せずそのまま返す扱いを維持する。Design 07 で固定し Design 09・10 の「変えないもの」で維持した判断であり、理由は、競合は Node の実行が失敗したことではなく記録が先に進んだことを表すためである。
- Design 10 で入れた、restart 経路の Conflict の有界再試行と、同一バッチの後続 Node の処理を継続する扱いを維持する。上の変更は、継続したまま精算の対象を発生元 Node へ限る側で解決する。

## 未確定・リスク

- 自動判断: なし。R-001〜R-011 と B-001〜B-014 は対応表に欠落なく相互に対応しており、誤り・不足・矛盾を見つけなかったため、Requirements・Behavior は変更していない。未決のまま残した要求は無く、`[DEFERRED]` で人間へ渡した件も無い。
