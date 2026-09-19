# Design 09

## 開始状態

- 差分の基準は base ブランチ `feat/issues/1203`、派生点 `020b097c`（design-07・design-08 と同じ）。作業ブランチ `feat/issues/1829` の HEAD は `020b097c` で、この変更はすべて作業ツリーの未コミット変更として存在する。
- 直前の Design は `docs/specs/issues-1829/design-08.md`。その「変える部分」12 件は実装済みで、対応する Thread 12 件はすべて resolved となった。見送り（`[DEFERRED]`）とした Thread、不成立（`[REJECTED]`）とした Thread はいずれも 0 件である。
- 今周で変える部分は、`[FIX_POLICY]` を持つ open Thread 6 件である。Thread `3ed091de-db2c-4935-b2e3-e7bd8e872cd5` は `[FIX_POLICY]` を 2 件持つ。今周は、現行コードを根拠として後から投稿された方の `[FIX_POLICY]` に従う。
- Requirements・Behavior は今周で変更していない。R-001〜R-014 / R-016〜R-018 と B-001〜B-016 / B-018〜B-020 の対応は保たれている（R-015 / B-017 は欠番）。対応表は現存するすべての Requirement ID 17 件を網羅し、参照先の Behavior ID はすべて本文に存在する。自動判断による修正は加えていない。

## 変える部分

- 初回 snapshot の受信前に terminal の出力 stream が完了した場合の再同期: 終了していない terminal の出力 stream が初回 snapshot の受信前に完了した場合も、再アタッチと snapshot 取得による再同期を行い、以後の出力を画面へ表示し、入力も受け付けるようにする。ここでの完了は、通常完了と、再同期を求める Closed の両方を指す。開始状態では、`src/lib/client.ts:371-375` が共有購読の完了で各 listener の `close(true, failure)` を呼ぶ。しかし `:456-461` は、初回 snapshot の受信前（`initialized` が false）には `rejectInitial` だけを行い、`onClosed` を呼ばない。通常完了では `failure` が無く、`:470` の `refreshClientOnDisconnect` の条件も満たさない。唯一の本番呼出元 `src/hooks/useTerminal.ts:523-535` は、エラーを表示した後も未解決の初回 snapshot を待ち続ける。終了済み terminal では再同期しない、という現在の扱いは変えない。根拠: Thread `3ed091de-db2c-4935-b2e3-e7bd8e872cd5`、R-011「terminal の入出力と、backend 状態の push による画面更新が動作する。」/ B-004、R-010「接続の回復後に、状態の再取得、push の購読の復旧、terminal の再同期が行われ、現在の状態が画面へ反映される。」/ B-013、design-08 の変える部分「終了していない terminal の stream が終了した場合は再同期する」。ルート: 委任
- push 再購読時の、状態の再取得と watcher 登録の順序: push を再購読するとき、状態の再取得が完了してから watcher の登録が完了するまでの間に監視対象が変更されても、その変更が画面へ反映されるようにする。開始状態では、`src/lib/client.ts:159-165` が push の先頭の resync で各 watcher を開始した直後に `refreshState` を呼ぶ。このとき `WatchFiles` / `WatchGitDirectory` の応答と `onReady` を待たず、`onReady` の後にも状態を再取得しない。`src/hooks/useAutomation.ts:136-150` は、新しい watcherId と一致する file-change しか反映しない。根拠: Thread `3d34ec6a-982f-4f39-bf82-3d2941c06a29`、R-010 / B-013「THEN 状態の再取得、push の購読の復旧、terminal の再同期が行われる AND 画面には現在の状態が反映される」。ルート: 委任
- attach の snapshot 構築中の共通ロック: ある terminal を attach して snapshot を構築している間も、別の attachment の出力 ack・detach・stream 解放が、その snapshot 構築の完了を待たないようにする。開始状態では、`src-tauri/src/usecase/terminal_surface/application.rs:350-357` が `attachment_cancellations` をロックしたまま snapshot 構築（`self.get(owner)`）と入力の activate を行う。同じロックを ack（`:400-406`）、detach（`:387-397`）、stream の Drop（`:94-111`）も取得する。同じ attachment_id で attach が成功した後に先行 attachment が終了・解放されても、成功した attachment の出力配信と入力受付が続く、という design-08 の条件は変えない。根拠: Thread `02cb02b7-1c8d-4e81-9265-73021d96aed1`、R-011 / B-004 / B-005。ルート: 委任
- 外部 RPC から受け取る識別子の長さ制限: 外部 RPC から受け取って保持する `subscription_id` と `stream_id` が上限長を超える場合、保持される前に拒否する。開始状態では、`src-tauri/src/adaptor/controller/api/client_service.rs:23-30,71-72,88-96` が受け取った ID をそのまま内側へ渡す。`src-tauri/src/domain/repository/watch_subscriptions.rs:38-53` は重複と件数だけを、`src-tauri/src/domain/terminal_surface/subscriptions.rs:43-57` は空白・重複・件数だけを検査して保持する。`stream_id` は `src-tauri/src/usecase/terminal_surface/subscriptions.rs:90-103` で attachment に保持される。同じ境界の `attachment_id` は、`src-tauri/src/adaptor/controller/api/client_stream.rs:79` で 128 byte 超を拒否している。クライアントが生成する通常の ID（UUID）による push 購読・terminal 購読・attach の結果は変えない。根拠: Thread `be0462ca-b6ae-42b0-89c6-ec158da7d35e`（Requirements・Behavior は識別子の長さを定めない）、R-002 / B-003、R-003 / B-004 / B-005。ルート: 委任（上限値の選定を含む）
- LocalApiServer の停止による共有 client token 失効のテスト: LocalApiServer の停止経路を通すテストで、停止前に有効だった共有 client token が停止後は認証を通らないことを検証する。`shutdown` から `revoke` を取り除く変更をすると、このテストが失敗するようにする。開始状態では、失効の本番契機は `src-tauri/src/infrastructure/local_api/server.rs:184-185` の `shutdown` 内の `revoke` である。`src-tauri/src/adaptor/controller/api/client_auth_test.rs:73` はテスト自身が `revoke` を呼んでおり、`src-tauri/src/infrastructure/local_api/server_test.rs:64-74` は停止経路を通すが、discovery file の削除しか検証しない。根拠: Thread `43f0db86-ec61-44a0-b98b-512d758d3c12`、R-006「token を失効させた後に送られる次の要求は、接続の確立時点で有効だった場合でも拒否される。」/ B-008、`docs/architecture/TEST.md`「『柔軟』のレイヤーも、テストを書ける範囲では書く。」。ルート: 委任
- master 認証の複数 token 用の間接層: master 認証の経路から、複数 token を受け取る collection（`AcceptedBearerTokens`）と `authenticated_with_tokens` の間接層を取り除く。開始状態では、`AcceptedBearerTokens` の構築は `src-tauri/src/adaptor/controller/api/mod.rs:56`（`authenticated` の単一 token）と `src-tauri/src/adaptor/controller/api/auth.rs:89`（テストの単一 token）だけである。`authenticated_with_tokens`（`mod.rs:59`）の呼出元も `authenticated` の一箇所だけである。HTTP local API（workflow / provider-lifecycle）の認証の受理・拒否の結果は変えない。これには、Authorization Bearer による受理と、既存の WebSocket handshake 時の Sec-WebSocket-Protocol による受理を含む。根拠: Thread `dff8d442-20c6-4066-9113-2b68dc4ca8e0`（要求違反ではない）、維持条件は R-012「CLI と provider hook が使う HTTP local API は、変更前と同じ経路・同じ認証で動作する。」/ B-014。ルート: 委任

## 固定するルート

今周で新たに固定する実装上の指定はない。design-01 で固定したルート 1〜11 を、design-07・design-08 に続いて今周も維持する。解除するルートはない。

1. （design-01 維持）Rust サーバのトランスポートに `connectrpc` crate を使う。範囲: クライアント向け RPC のサーバ実装。粒度: 使う crate の指定のみで、配置と構成は委任。
2. （design-01 維持）Web クライアントのトランスポートに `@connectrpc/connect-web` を使う。範囲: renderer のクライアント実装。粒度: 使うパッケージの指定のみ。
3. （design-01 維持）自前の RPC 層である `src/lib/clientSocket.ts` と `src/generated/client_transport.ts` を削除する。範囲: この 2 ファイル。
4. （design-01 維持）desktop の Tauri IPC 通信経路を廃止する。対象は `src/lib/desktopClientSocket.ts` の `DesktopClientSocket` と、`attach_desktop_client` / `send_desktop_client_frame` / `detach_desktop_client` / `admit_client_command`。
5. （design-01 維持）backend 状態の push を Connect の server streaming に載せる。粒度: トランスポート方式の指定のみ。
6. （design-01 維持）terminal を、出力 = server streaming、入力 = unary に分ける。粒度: RPC 種別の指定のみ。入出力の対応付けとフロー制御の扱いは委任。
7. （design-01 維持）起動・切替中に変更要求を拒否する受付制御を廃止する。対象は `admit_client_command`、`DaemonSupervisionUsecase::admit_client_command`、`Supervision::client_command_admitted`。
8. （design-01 維持）結果を確認できない変更要求を扱う仕組みを、クライアント側・サーバ側ともに廃止する。サーバ側の削除対象は `src-tauri/src/domain/client_operation/`、`src-tauri/src/usecase/client_operation.rs`、`src-tauri/src/adaptor/controller/api/client_operation.rs`。
9. （design-01 維持）`ClientConnectionBanner` を廃止する。範囲: `src/components/ClientConnectionBanner.tsx` と、`src/App.tsx` でそれを描画する箇所。
10. （design-01 維持）各 UI の `onUncertain` / `retryClientOperation` / `dismissClientOperation` を取り除く。範囲: `CreateWorktreeModal` / `WorkspaceList` / `DeleteWorktreeDialog` / `SettingsModal` / `NodeContentView` / `useNotionSettings` / `useProviderAvailabilitySettings` / `useAppSettings` / `ProviderAvailabilitySettings`。
11. （design-01 維持）判断③・判断⑧を改訂する場所は、判断の正本である GitHub milestone 77 の説明文だけとする。範囲: milestone の説明文。

## 変えないもの

- 既存の spec（`docs/specs/issues-1199/`、`docs/specs/issues-1200/`、`docs/specs/issues-1201/`、`docs/specs/issues-1202/`、`docs/specs/issues-1203/`）。`docs/specs/issues-1201/behavior.md` の B-009〜B-031 と `docs/specs/issues-1201/requirements.md` も含め、一切変更しない。理由: 完了した spec は改訂の対象にしないため。
- R-014 の対象範囲（判断③・判断⑧）と、固定するルート 11（改訂する場所は GitHub milestone 77 の説明文だけ）。理由: 判断の改訂範囲を広げないため。
- 固定するルート 5（push = server streaming）・6（terminal の出力 = server streaming、入力 = unary）と、R-001（desktop が backend を呼ぶ通信は Tauri IPC を経由しない）。理由: 今周の terminal・push の変更は、RPC 種別とトランスポートを保ったまま行うため。
- design-01〜design-08 の記載。理由: 過去の周の Design は書き換えないため。
- Requirements の Non-goals と、design-02 の「変えないもの」に挙げた条件。design-02 の当該節をそのまま維持する。理由: 今周の決定は、どれもこれらの範囲を変えないため。

## 未確定・リスク

- この周で Requirements・Behavior に自動判断による修正は加えていない。Assumptions に「自動判断」「自動判断: 未決」として残した要求、`[DEFERRED]` で人間へ渡した件、不成立として扱った件はいずれも無い。
