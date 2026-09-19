# Design 07

## 開始状態

- 差分の基準は base ブランチ `feat/issues/1203`、派生点 `020b097c`。作業ブランチ `feat/issues/1829` の HEAD は `020b097c` で、この変更はすべて作業ツリーの未コミット変更として存在する。design-01〜design-06 の開始状態に記した基準（`main`・`00b57d77`）は書き換えず、この周からこの基準を使う。
- 直前の Design は `docs/specs/issues-1829/design-06.md`。その「変える部分」3 件はすべて実装済みである。対応する Thread のうち `56c606f2-be6d-4959-8522-9e2abe18501b` / `0b5bf893-ac5c-4840-a73d-a99fe52f9d18` は resolved となった。`eb315d10-ffa7-4ef4-948c-f8b82f22426b` は `[FIX_POLICY]` を持ったまま open で、これが今周で変える部分である。見送り（`[DEFERRED]`）とした Thread、不成立（`[REJECTED]`）とした Thread はいずれも 0 件である。
- Requirements・Behavior は今周で変更していない。R-001〜R-014 / R-016〜R-018 と B-001〜B-016 / B-018〜B-020 の対応は保たれている（R-015 / B-017 は欠番）。対応表は現存するすべての Requirement ID 17 件を網羅し、参照先の Behavior ID はすべて本文に存在する。自動判断による修正は加えていない。

## 変える部分

- renderer の長寿命 server stream と unary RPC の接続構成について、受入条件を満たす状態の確立: 開始状態では、`src/lib/client.ts:333-382` が client ごとに `SubscribeTerminalSurfaces`（`proto/client.proto:3720` の server stream）を 1 本だけ共有する。各 attachment は `:410` の `AttachTerminalSurface`（unary）で登録し、push は `:153-155` の `SubscribePush` 1 本である。このため、長寿命 stream は pane 数によらず push と terminal の 2 本になった。ただし、Thread の受入条件が現行実装で満たされることは示されていない。受入条件は、保持可能な最大数の pane の terminal と push の stream が同時に開いている状態で、各 terminal への入力が届き、出力が継続して表示され、状態の再取得を含む unary RPC が接続待ちで止まらないことである。示すべき対象は、renderer が実際に動く macOS の WebView と、平文の `http://127.0.0.1:{port}` 接続である。`tests/client-streams.spec.ts` は `tests/helpers/tauri-mock.ts` の模擬 backend を使うテストコードで、この観測の根拠として扱われていない。この状態を、現行実装が対象環境でこの受入条件を満たすことを示せる形へ変える。満たさない場合は、満たすように接続構成を変える。固定するルート 5・6 と R-001 は維持する。根拠: Thread `eb315d10-ffa7-4ef4-948c-f8b82f22426b`、R-011「desktop の機能は、変更前と同じ結果が画面へ反映される。terminal の入出力と、backend 状態の push による画面更新が動作する。」/ B-004 / B-005。ルート: 委任

## 固定するルート

今周で新たに固定する実装上の指定はない。design-01 で固定したルート 1〜11 を今周も維持し、解除するルートはない。

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
- 固定するルート 5（push = server streaming）・6（terminal 出力 = server streaming、入力 = unary）と R-001（desktop が backend を呼ぶ通信は Tauri IPC を経由しない）。理由: 接続構成は、これらを保ったまま変えるため。
- design-01〜design-06 の記載。理由: 差分の基準の変更はこの周の開始状態へ記載することとし、過去の周の Design は書き換えないため。
- Requirements の Non-goals と design-02 の「変えないもの」に挙げた条件。design-02 の当該節をそのまま維持する。理由: 今周の決定は、いずれもこれらの範囲を変えないため。

## 未確定・リスク

- 対象環境の macOS の WebView で、同一ホストに開ける接続枠の厳密な数は未計測である。開始状態の構成は、長寿命 stream 2 本と unary を同時に扱えるだけの枠があることを前提にしている。この前提が外れると、R-011 / B-004 / B-005 を満たせない可能性がある。
- この周で Requirements・Behavior に自動判断による修正は加えていない。Assumptions に「自動判断: 未決」として残した要求、`[DEFERRED]` で人間へ渡した件、不成立として扱った件はいずれも無い。
