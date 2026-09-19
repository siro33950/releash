# Design 03

## 開始状態

- 直前の Design は `docs/specs/issues-1829/design-02.md`。その「変える部分」18 件はすべて実装済みで、作業ツリーの未コミット変更として存在する。対応する 18 件の Thread（`79699b85` / `f320ceb6` / `e05f1bb6` / `5df5add7` / `461f8d72` / `ac0db313` / `e7421888` / `fd6bcb77` / `c81ba86e` / `48444112` / `f501162b` / `f87033b6` / `959f5378` / `e6c5edb8` / `b6c9cd32` / `a061d437` / `0003cc34` / `cc9a7c59`）は、この周までにすべて resolved となった。見送り（`[DEFERRED]`）とした Thread、不成立（`[REJECTED]`）とした Thread はいずれも 0 件である。
- 差分の基準は base ブランチ `main`、派生点 `00b57d77`。作業ブランチ `feat/issues/1829` は `020b097c`。
- open Thread は 9 件で、すべて `[FIX_POLICY]` を持つ。この 9 件が今周で変える部分である。
- Requirements・Behavior は今周で R-018 と B-020、および対応表の `| R-018 | B-020 |` が追加された。他の Requirement・Behavior に変更はない。R-001〜R-014 / R-016〜R-018 と B-001〜B-016 / B-018〜B-020 の対応は保たれており（R-015 / B-017 は欠番）、自動判断による修正は加えていない。

## 変える部分

- desktop 設定の再適用対象の判定の所有: `scripts/generate-client-protocol.mjs:54` が `update_app_settings` / `update_crash_reporting` / `update_performance_telemetry` の 3 名を `includes` で判定し、生成物から `src/lib/client.ts:355-363` の `applyClientDesktopSettings`（`GetServerInfo` の再取得と `apply_desktop_settings`）を呼ぶ状態を、対象判定を Rust が所有し renderer と生成器に対象コマンド名の列挙が無い形へ変える。根拠: Thread `3f84c6db-a9a0-4015-a0f1-5eabb09236b5`、AGENTS.md「全てのアプリケーションロジックは Rust に置く。例外なし。」、`docs/architecture/README.md:32`「同じ操作の実装は 1 つに集約する。」、R-001 / B-001。ルート: 委任
- 接続断ではない ConnectError での共有接続の破棄: `scripts/generate-client-protocol.mjs:57` が出力する `invokeClient` の catch が CommandError detail の無い全 ConnectError で `refreshClient(client)` を呼び、`src/lib/client.ts:99-110` が共有 `connectionAbort` を abort して進行中の RPC / stream を中断する状態（`src/lib/client.ts:262-270` の `watchClient` にも同じ判定がある）を、当該要求だけが失敗し共有接続と他の進行中要求が維持される形へ変える。根拠: Thread `2788fa53-755e-4d01-b466-3636eaf89299`、R-011「desktop の機能は、変更前と同じ結果が画面へ反映される。」/ B-001。R-010 / B-013 が定めるのは接続が切れた場合の再接続と再取得である。ルート: 委任
- terminal の attachment 上限超過の理由の伝達: `src-tauri/src/adaptor/controller/api/client_stream.rs:38-42` が detail 無しの `resource_exhausted` を返し、`src/lib/errorMessage.ts:4` が ConnectError を一律に固定文へ置換するため、上限超過という業務上の拒否理由が利用者へ届かない状態を、理由が届く形へ変える。根拠: Thread `568056fb-22cf-4cdb-8af1-f62db350ca81`、R-011 / B-004。R-009「通信の内部状態（未送信・結果不明・再接続中）を利用者へ提示する UI が存在しない。」/ B-012 が対象とする内部状態は引き続き提示しない。ルート: 委任
- push payload の購読者ごとの再変換: `src/lib/client.ts:140-149` が一致した listener ごとに `decodeClientPush` を呼び、`src/lib/clientProtocol.ts:11-31` が毎回 schema 探索と `toJson` と `clientJson` を実行する状態を、受信 event ごとに 1 回の復号・変換とし listener 数に比例させない形へ変える。根拠: Thread `a953420f-434d-4275-82c8-b0bce7e4711c`、AGENTS.md「full-retention / full-recompute 経路を増やしていないか。」、R-002 / B-003。ルート: 委任
- 監視 RPC の進行中要求上限: `src-tauri/src/adaptor/controller/api/client.rs:63-73` の `execute` だけが `request_limit` の permit を取得し、`client_service.rs:79-116` の `WatchFiles` / `WatchGitDirectory` が `client.rs:106-127` の `watch` へ permit 無しで入り `spawn_blocking` する状態を、blocking へ投入する前に `execute` と同じ上限を取得し、上限超過時は投入前に拒否する形へ変える。根拠: Thread `247e78fa-20d5-4861-9e30-8bf64593ddd4`、R-011 / B-001。ルート: 委任
- `attachClientStream` の初回受信失敗の検証: `src/lib/client.ts:302-317` が初回 `iterator.next` の失敗を「CommandError detail 付き ConnectError」「detail 無し ConnectError」「snapshot 前の正常終了」の 3 分岐で異なる失敗値へ変換し、`src/hooks/useTerminal.ts:535-539,591-602` がその型で表示を分ける一方、初回受信が成功する経路しか検証されていない状態を、3 分岐それぞれで呼び出し元へ渡る失敗値の型と code / message を検証できる形へ変える。根拠: Thread `1c099d8e-13c9-42cb-b0da-8a1a44bea618`、R-003 / R-011 / B-004。ルート: 委任
- push stream 単独の終了・失敗からの回復の検証: `src/lib/client.ts:117-157` の回復経路のうち、`SubscribePush` の終了・失敗を入力とする分岐が検証されておらず、`src/lib/client.test.ts` の回復検証が `refreshClient` の直接呼び出しか別 RPC の失敗を起点としている状態を、push stream だけが終了・失敗した状態を起点に再購読と状態の再取得を検証できる形へ変える。根拠: Thread `4e157fb3-4114-449c-9947-30322a24a455`、R-010 / B-013。ルート: 委任
- 構造化業務エラーの変換契約の検証: `invokeClient` が CommandError detail 付き ConnectError を元の code / message を持つ拒否値へ戻す契約を持つ一方、`src/lib/client.test.ts:13-24` が成功値と要求パスだけを検証し、失敗例がいずれも detail を伴わない状態を、実 adapter を通る経路でこの契約を検証し detail 無しの ConnectError と区別できる形へ変える。根拠: Thread `8fb3a752-eaf8-4ba5-8672-8120eaf24fa2`、R-011 / B-001。ルート: 委任
- 要求の正本の spec 項目の取り下げ: GitHub Issue #1829 の本文に「`docs/specs/issues-1201/behavior.md` B-009〜B-031 を改訂する」という spec 項目と、「設計で具体化する点」の「それに伴う B-009〜B-031 の改訂範囲」が残り、Requirements の Non-goals および design-02 の「変えないもの」と矛盾する状態を、正本から当該記述を取り下げて解消する。根拠: Thread `17996b3b-7ead-4227-a99e-94d0933bdb20`、R-018「要求の正本である Issue #1829 の本文は、`docs/specs/issues-1201/behavior.md` の B-009〜B-031 の改訂を求めない。」/ B-020。ルート: 委任

## 固定するルート

今周で新たに固定する実装上の指定はない。design-01 で固定したルート 1〜11 を今周も維持する。解除するルートはない。

1. （design-01 維持）Rust サーバのトランスポートに `connectrpc` crate を使う。範囲: クライアント向け RPC のサーバ実装。粒度: 使用する crate の指定のみで、配置と構成は委任。
2. （design-01 維持）Web クライアントのトランスポートに `@connectrpc/connect-web` を使う。範囲: renderer のクライアント実装。粒度: 使用するパッケージの指定のみ。
3. （design-01 維持）自前 RPC 層として `src/lib/clientSocket.ts` と `src/generated/client_transport.ts` を削除する。範囲: この 2 ファイル。
4. （design-01 維持）desktop の Tauri IPC 通信経路を廃止する。対象は `src/lib/desktopClientSocket.ts` の `DesktopClientSocket` と、`attach_desktop_client` / `send_desktop_client_frame` / `detach_desktop_client` / `admit_client_command`。
5. （design-01 維持）backend 状態の push を Connect の server streaming に載せる。粒度: トランスポート方式の指定のみ。
6. （design-01 維持）terminal を出力 = server streaming、入力 = unary に分解する。粒度: RPC 種別の指定のみ。入出力の対応付けとフロー制御の扱いは委任。
7. （design-01 維持）起動・切替中に変更要求を拒否する受付制御を廃止する。対象は `admit_client_command`、`DaemonSupervisionUsecase::admit_client_command`、`Supervision::client_command_admitted`。
8. （design-01 維持）結果を確認できない変更要求を扱う仕組みを、クライアント側・サーバ側ともに廃止する。サーバ側の削除対象は `src-tauri/src/domain/client_operation/`、`src-tauri/src/usecase/client_operation.rs`、`src-tauri/src/adaptor/controller/api/client_operation.rs`。
9. （design-01 維持）`ClientConnectionBanner` を廃止する。範囲: `src/components/ClientConnectionBanner.tsx` と `src/App.tsx` の描画。
10. （design-01 維持）各 UI の `onUncertain` / `retryClientOperation` / `dismissClientOperation` を除去する。範囲: `CreateWorktreeModal` / `WorkspaceList` / `DeleteWorktreeDialog` / `SettingsModal` / `NodeContentView` / `useNotionSettings` / `useProviderAvailabilitySettings` / `useAppSettings` / `ProviderAvailabilitySettings`。
11. （design-01 維持）判断③・判断⑧の改訂先は、判断の正本である GitHub milestone 77 の説明文だけとする。範囲: milestone 説明文。

## 変えないもの

- 既存 spec（`docs/specs/issues-1199/`、`docs/specs/issues-1200/`、`docs/specs/issues-1201/`、`docs/specs/issues-1202/`、`docs/specs/issues-1203/`）。`docs/specs/issues-1201/behavior.md` の B-009〜B-031 と `docs/specs/issues-1201/requirements.md` を含め、一切変更しない。正本 Issue #1829 の本文の改訂は、この範囲を変えずに矛盾を解消するための手段である。理由: 終わった spec を改訂対象にしないため。
- R-014 の対象範囲（判断③・判断⑧）と、固定するルート 11（改訂先は GitHub milestone 77 の説明文だけ）。理由: 正本の改訂は B-009〜B-031 に関する記述の取り下げに限り、判断の改訂範囲を広げないため。
- Requirements の Non-goals と design-02 の「変えないもの」に挙げた条件。design-02 の当該節をそのまま維持する。理由: 今周の決定はいずれもこれらの範囲を変えないため。

## 未確定・リスク

なし。この周で Requirements・Behavior に自動判断による修正は加えておらず、Assumptions に「自動判断: 未決」として残した要求、`[DEFERRED]` で人間へ渡した件、不成立として扱った件はいずれも無い。
