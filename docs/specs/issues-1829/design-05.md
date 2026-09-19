# Design 05

## 開始状態

- 直前の Design は `docs/specs/issues-1829/design-04.md`。その「変える部分」3 件はすべて実装済みで、作業ツリーの未コミット変更として存在する。対応する 3 件の Thread（`30ad2783-5700-4812-9e30-a58499c57b34` / `decf37c7-f758-459a-9cb1-64c19171c6c7` / `f90dc6c1-6580-48dd-b031-d7f55c58aa9c`）は、この周までにすべて resolved となった。見送り（`[DEFERRED]`）とした Thread、不成立（`[REJECTED]`）とした Thread はいずれも 0 件である。
- 差分の基準は base ブランチ `main`、派生点 `00b57d77`。作業ブランチ `feat/issues/1829` は `020b097c`。
- open Thread は 2 件で、いずれも `[FIX_POLICY]` を持つ。この 2 件が今周で変える部分である。
- Requirements・Behavior は今周で変更していない。R-001〜R-014 / R-016〜R-018 と B-001〜B-016 / B-018〜B-020 の対応は保たれており（R-015 / B-017 は欠番）、対応表は現存するすべての Requirement ID 17 件を網羅し、参照先の Behavior ID はすべて本文に存在する。自動判断による修正は加えていない。

## 変える部分

- 変更要求の成功応答と desktop 設定の再取得の分離: `src/lib/client.ts:64-74` の transport interceptor が `await next(request)` で成功応答を受け取った後に応答ヘッダ `releash-desktop-settings-changed` を契機として `applyClientDesktopSettings` を await し、`src/lib/client.ts:388-393` の追加 `GetServerInfo` が throw すると成功応答を返さずに例外が interceptor から伝播するため、`src/hooks/useAppSettings.ts:84-101` が catch で `setError` し `setConfig` へ進まない状態を、確定済みの保存結果が画面へ反映される形へ変える。追加取得が detail 無しの `Unavailable` または `Unknown`＋`TypeError` であった場合に `src/lib/client.ts:123-134` の `refreshClientOnDisconnect` が共有接続を破棄し、無関係な進行中 RPC と terminal の出力まで中断される点も併せて変える。根拠: Thread `4a59ffe6-af6e-4250-bc73-db7f7051a990`、R-011「desktop の機能は、変更前と同じ結果が画面へ反映される。」/ B-001。ルート: 委任
- terminal の出力 stream の途中完了からの再同期: `src/lib/client.ts:353-364` が `iterator.next()` の `done` を無通知で break し `onClosed` を catch でしか呼ばず、`src-tauri/src/usecase/terminal_surface/application.rs:94-110` の `resynchronize` が `apply_snapshot` で欠番を覆えない場合に `Exit` を出さず `None` を返し（`:145-147`・`:206-209` の `Closed` 判定も同様）、`src-tauri/src/adaptor/controller/api/client_stream.rs:61-67` の `unfold` がその `None` を stream の通常完了へ写すため、terminal プロセスが稼働中でも出力 stream だけが正常終了し、`src/hooks/useTerminal.ts:528-535` の `onClosed`（唯一の再同期契機）が呼ばれない状態を、再アタッチと snapshot 取得による再同期が行われる形へ変える。終了済み terminal で再同期しない現在の扱いは変えない。根拠: Thread `a0dcdc6b-d278-4d2a-b817-d37004182fe4`、R-011 / B-004。復旧の経路は R-010「接続の回復後に、状態の再取得、push の購読の復旧、terminal の再同期が行われ、現在の状態が画面へ反映される。」/ B-013 が定める。ルート: 委任

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
- 終了済み terminal では再同期を行わない現在の扱い。理由: 今周の対象は稼働中の terminal で出力 stream だけが途中完了した場合に限られるため。
- Requirements の Non-goals と design-02 の「変えないもの」に挙げた条件。design-02 の当該節をそのまま維持する。理由: 今周の決定はいずれもこれらの範囲を変えないため。

## 未確定・リスク

なし。この周で Requirements・Behavior に自動判断による修正は加えておらず、Assumptions に「自動判断: 未決」として残した要求、`[DEFERRED]` で人間へ渡した件、不成立として扱った件はいずれも無い。
