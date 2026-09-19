# Design 04

## 開始状態

- 直前の Design は `docs/specs/issues-1829/design-03.md`。その「変える部分」9 件はすべて実装済みで、作業ツリーの未コミット変更として存在する。対応する 9 件の Thread（`3f84c6db` / `2788fa53` / `568056fb` / `a953420f` / `247e78fa` / `1c099d8e` / `4e157fb3` / `8fb3a752` / `17996b3b`）は、この周までにすべて resolved となった。見送り（`[DEFERRED]`）とした Thread、不成立（`[REJECTED]`）とした Thread はいずれも 0 件である。
- 差分の基準は base ブランチ `main`、派生点 `00b57d77`。作業ブランチ `feat/issues/1829` は `020b097c`。
- open Thread は 3 件で、すべて `[FIX_POLICY]` を持つ。この 3 件が今周で変える部分である。
- Requirements・Behavior は今周で変更していない。R-001〜R-014 / R-016〜R-018 と B-001〜B-016 / B-018〜B-020 の対応は保たれており（R-015 / B-017 は欠番）、対応表は現存するすべての Requirement ID を網羅し、参照先の Behavior ID はすべて本文に存在する。自動判断による修正は加えていない。

## 変える部分

- push 再購読時の監視の再登録: `src/lib/client.ts:158` が新しい `SubscribePush` の初回 resync で watcher を一度だけ再登録し、`src/lib/client.ts:288-289` の catch が `onError` と `refreshClientOnDisconnect` を呼ぶだけで、`src/lib/client.ts:123-134` が `Unavailable` と `Unknown`＋`TypeError` しか再接続の契機にせず `ResourceExhausted` を含まないため、`src-tauri/src/domain/repository/watch_subscriptions.rs:35-41` の全購読合計 64 件の上限と `src-tauri/src/usecase/watcher.rs:119-125` の `spawn_blocking` による旧購読の非同期な解放が重なって再登録が拒否されると、その後に枠が解放されても再登録へ進む経路が無い状態を、枠の解放後に監視が再登録される形へ変える。根拠: Thread `30ad2783-5700-4812-9e30-a58499c57b34`、R-010「接続の回復後に、状態の再取得、push の購読の復旧、terminal の再同期が行われ、現在の状態が画面へ反映される。」/ B-013。ルート: 委任
- terminal の出力 stream の初回受信失敗からの再同期: `src/lib/client.ts:324-335` の `attachClientStream` が最初の `iterator.next` の失敗で abort して再 throw するだけで `refreshClientOnDisconnect` も `onClosed` も呼ばず、`src/hooks/useTerminal.ts` の `recoverAttachment` の契機が開始後の `onClosed`・stream item の適用失敗・overflow・`onClientConnection` に限られるため、共有接続と `SubscribePush` が正常なまま terminal 要求だけが切れた場合にどの契機も発生しない状態を、再アタッチと snapshot 取得が自動で行われる形へ変える。根拠: Thread `decf37c7-f758-459a-9cb1-64c19171c6c7`、R-010 / B-013。ルート: 委任
- 進行中要求の上限超過の拒否理由の伝達: `src-tauri/src/adaptor/controller/api/client.rs:63-69` の `request_permit` が CommandError detail を持たない `resource_exhausted` を返し、`src/generated/client_commands.ts` の `invokeClient` が detail の無い `ConnectError` をそのまま再 throw して `src/lib/errorMessage.ts:4` が固定文へ置換するため、上限超過という業務上の拒否理由が利用者へ届かない状態を、理由が届く形へ変える。根拠: Thread `f90dc6c1-6580-48dd-b031-d7f55c58aa9c`、R-011「desktop の機能は、変更前と同じ結果が画面へ反映される。」/ B-001。R-009「通信の内部状態（未送信・結果不明・再接続中）を利用者へ提示する UI が存在しない。」/ B-012 が対象とする内部状態は引き続き提示しない。ルート: 委任

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

- 既存 spec（`docs/specs/issues-1199/`、`docs/specs/issues-1200/`、`docs/specs/issues-1201/`、`docs/specs/issues-1202/`、`docs/specs/issues-1203/`）。`docs/specs/issues-1201/behavior.md` の B-009〜B-031 と `docs/specs/issues-1201/requirements.md` を含め、一切変更しない。理由: 終わった spec を改訂対象にしないため。
- R-014 の対象範囲（判断③・判断⑧）と、固定するルート 11（改訂先は GitHub milestone 77 の説明文だけ）。理由: 判断の改訂範囲を広げないため。
- Requirements の Non-goals と design-02 の「変えないもの」に挙げた条件。design-02 の当該節をそのまま維持する。理由: 今周の決定はいずれもこれらの範囲を変えないため。

## 未確定・リスク

なし。この周で Requirements・Behavior に自動判断による修正は加えておらず、Assumptions に「自動判断: 未決」として残した要求、`[DEFERRED]` で人間へ渡した件、不成立として扱った件はいずれも無い。
