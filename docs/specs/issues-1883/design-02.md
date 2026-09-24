# Design 02

## 開始状態

- 差分の基準は base `main` からの派生点 `42be41e0`（`refactor(error): 失敗の分類をエラーの値に持たせる (#1880) (#1919)`）。branch は `feat/issues/1883`。
- 直前の Design は `docs/specs/issues-1883/design-01.md`。同 Design の「変える部分」は未コミットの作業ツリーに実装済みである（`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`、`src-tauri/src/adaptor/controller/api/client.rs`、`client_service.rs`、`client_test.rs`、`src-tauri/src/adaptor/controller/client/dispatch.rs`、`src-tauri/src/adaptor/controller/command/client_test.rs`）。この実装を開始状態とする。
- `docs/specs/issues-1883/requirements.md` と `docs/specs/issues-1883/behavior.md` は前の周から変更していない。R-001〜R-006 と B-001〜B-006 の対応に誤り・不足・矛盾は無い。
- この周までに解消・見送りとなった Thread は無い。`[REJECTED]`・`[DEFERRED]` とした Thread も無い。open Thread は 4 件で、いずれも `[FIX_POLICY]` が付いている。
- 本文で引く行番号は、この作業ツリーの現在の実装で確認したものである。

## 変える部分

- worktree の変更の排他が取り消しで先に解ける点の修正: 取り消しで単発の呼び出しが終わった後も、開始済みの同期処理が続く間は同じ worktree の削除・変更操作が受理されないようにする。開始状態では `dispatch_admitted` が `WorktreeMutationGuard` を `run_until_cancelled` と同じ async block に保持するため（`dispatch.rs:130-146`）、取り消しで handler の future が drop されると `WorktreeMutationGuard::drop` が `finish_mutation()` と通知を行い（`usecase/worktree_operation.rs:111-116`）、`spawn_blocking` 上で走り続ける git 処理と削除待機（同 `86-108`）が並行しうる。根拠: Thread `4b0bda2e-7a17-4c03-9097-3a45a67716f6`（`[FIX_POLICY]` 受入条件「期限切れまたは client の中断で単発の呼び出しが DEADLINE_EXCEEDED / CANCELLED で終わった後も、開始済みの同期処理が続く間は同じ worktree の削除・変更操作が受理されない。同時実行の枠は R-004・B-004 のとおり呼び出しの終了と同時に解放される」）。ルート: 委任。
- 取り消しの停止判定の所在の修正: 取り消しの伝搬と停止の判定、枠の解放を `adaptor/controller/api` が持ち、`controller/client` の command dispatch が API の期限・切断の規則を持たないようにする。開始状態では `client.rs:153-154` が token の生成と `drop_guard` だけを持ち、停止判定の `run_until_cancelled` と `FailureKind::Cancelled` への変換は `dispatch.rs:119-146` にある。根拠: Thread `a84755ef-83c0-41ab-ade7-9acc2412316a`（`[FIX_POLICY]` 受入条件「取り消しの伝搬と停止の判定、枠の解放が adaptor/controller/api にあり、controller/client の command dispatch が API の期限・切断の規則を持たない。B-003・B-005 の観測結果は変わらない」）。Design 01 の固定するルート「取り消しの伝搬と枠の解放は `adaptor/controller/api` が持つ」に対する不一致である。ルート: 委任。
- `tokio-util` の feature 指定の修正: `src-tauri/Cargo.toml:63` の `tokio-util = { version = "0.7", features = ["rt"] }` を、実際に使う `sync::CancellationToken` に必要な最小の指定にする。開始状態の `src-tauri/src` の参照は `tokio_util::sync::CancellationToken`（`client.rs:153`、`dispatch.rs:115,120`、`client_test.rs`）だけで、`rt` は使っていない。根拠: Thread `a8c64bdd-6fa1-4cd7-b8f7-9cde85c86171`（`[FIX_POLICY]` 受入条件「`src-tauri/Cargo.toml` の `tokio-util` が、実際に使う `sync::CancellationToken` に必要な最小の指定になり、`Cargo.lock` の `tokio-util` から `futures-util` への依存辺が増えない。ビルドとテストが通る」）。ルート: 委任。
- `StopWatching` の枠の解放の回帰検証の追加: `StopWatching` の呼び出しを中断したとき、blocking 処理の完了を待たずに同時実行の枠が解放されることを回帰テストで確認できるようにする。開始状態では permit を handler 側で保持する形へ変えた 2 経路のうち、`watch` にだけ中断時の枠の解放を確認するテストがあり、`StopWatching` には無い。根拠: Thread `4ba2c309-f6e3-4481-9c75-f186ab2b83d4`（`[FIX_POLICY]` 受入条件「`StopWatching` の呼び出しを中断したとき、blocking 処理の完了を待たずに同時実行の枠が解放されることを回帰テストで確認できる」）。ルート: 委任。

Requirements・Behavior はこの周で変更していないため、要求の追加・削除・統合・緩和に対応する変更は無い。

## 固定するルート

この周に人間が新たに固定した実装上の指定は無い。4 件の `[FIX_POLICY]` はいずれも「ルートは委任」であり、修正の手段・配置・テストの形を指定していない。

Design 01 で固定し、この周も維持するルート（Design 01「固定するルート」の各項目を、解除せずそのまま維持する）:

- 方針は gRPC の deadline に従う。
- daemon の既定の期限は 120 秒。
- client が指定した期限に daemon 側の上限・下限を設けない。
- 期限切れと client の中断は 1 つの取り消しの仕組みで伝え、手段は `tokio_util::sync::CancellationToken`、`src-tauri/Cargo.toml` に直接依存として追記する。
- 期限切れは `DEADLINE_EXCEEDED`、中断は `CANCELLED`。既存の `FailureKind` → Connect エラーコードの変換（`adaptor/protocol/connect.rs:26-47`）を使い、新しい分類を作らない。
- 規則の所有: 期限の適用は connectrpc の `DeadlinePolicy`、期限切れ・中断の分類は `domain::failure::FailureKind`、取り消しの伝搬と枠の解放は `adaptor/controller/api` が持つ。ドメインモデルを新設しない。
- 切り離した task の終わり方を失敗の分類へ写す処理を `client.rs` の 1 か所にまとめる。`repository/mod.rs` の同型（`run_blocking` / `run_repository_state`）は対象外。
- 購読の stream を開いた後は期限の対象にしない（`DeadlinePolicy` の stream の item への適用を有効にしない）。

## 変えないもの

Design 01「変えないもの」を維持する。この周に人間が新たに維持すると決めた条件は無い。

- 画面の React が付ける既定の期限 120 秒（`src/lib/client.ts:70`）を変更しない。
- Tauri のシェルが付ける既定の期限 30 秒と生存監視の 5 秒（`src-tauri/src/adaptor/gateway/desktop_client.rs:19,53`）を変更しない。
- client が指定した期限を daemon 側で変更しない（上限・下限を設けない）。
- 購読の stream を開いた後は期限の対象にしない。
- 既に同期処理として動き始めた処理の内側には取り消しを届けない。
- 期限切れ・中断の分類に新しい仕組みを作らない。

## 未確定・リスク

- worktree の変更の排他（Thread `4b0bda2e`）と同時実行の枠（R-004・B-004）は、開始状態では同じ async block の寿命に束ねられている。前者は開始済みの `spawn_blocking` が終わるまで保持され続ける必要があり、後者は呼び出しの終了と同時に解放される必要がある。両者の寿命を分けられない場合、どちらかの受入条件を満たせない。
- この周で自動判断した箇所は無い。未決のまま残した要求も無い（Requirements の Assumptions / Open Questions は「なし」）。`[DEFERRED]` で人間へ渡した件も無い。
