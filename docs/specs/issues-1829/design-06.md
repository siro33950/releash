# Design 06

## 開始状態

- 直前の Design は `docs/specs/issues-1829/design-05.md`。その「変える部分」2 件はすべて実装済みで、作業ツリーの未コミット変更として存在する。対応する 2 件の Thread（`4a59ffe6-af6e-4250-bc73-db7f7051a990` / `a0dcdc6b-d278-4d2a-b817-d37004182fe4`）は、この周までにすべて resolved となった。見送り（`[DEFERRED]`）とした Thread、不成立（`[REJECTED]`）とした Thread はいずれも 0 件である。
- 差分の基準は base ブランチ `main`、派生点 `00b57d77`。作業ブランチ `feat/issues/1829` は `020b097c`。
- open Thread は 3 件で、いずれも `[FIX_POLICY]` を持つ。この 3 件が今周で変える部分である。
- Requirements・Behavior は今周で変更していない。R-001〜R-014 / R-016〜R-018 と B-001〜B-016 / B-018〜B-020 の対応は保たれている（R-015 / B-017 は欠番）。対応表は現存するすべての Requirement ID 17 件を網羅し、参照先の Behavior ID はすべて本文に存在する。自動判断による修正は加えていない。

## 変える部分

- GetServerInfo の要求上限拒否と daemon 接続の喪失の区別: `src-tauri/src/adaptor/gateway/desktop_client.rs:50-63` の監視 task は、5 秒ごとの `GetServerInfo` が返す `ConnectError` を種別を問わず `failure` へ保存して終了する。その結果 `:74-76` の `connected()` が false になる。`GetServerInfo`（`src-tauri/src/adaptor/controller/api/client_service.rs:6`）は 64 枠の共有 `request_permit`（`src-tauri/src/adaptor/controller/api/client.rs:63-74`）を取り、枠が埋まると `CLIENT_REQUEST_LIMIT` の `ResourceExhausted` を返す。このため要求枠が一時的に埋まっただけで、`src-tauri/src/usecase/daemon_supervision.rs:375` → `src-tauri/src/domain/daemon_supervision.rs:168-172` が Ready / Restoring を Starting へ戻す。拒否が 30 秒続けば `src-tauri/src/usecase/daemon_supervision.rs:378-390` の `startup_interruption` から `terminate_and_wait` へ進む。この状態を、要求上限の拒否だけでは監督 phase・接続・復元済み状態を失わない形へ変える。daemon が実際に応答しなくなった場合は、変更前と同じく接続断として扱う。根拠: Thread `56c606f2-be6d-4959-8522-9e2abe18501b`、R-013「daemon の起動・監視・停止と `DaemonBoundary` の画面は、変更前と同じ振る舞いである。」/ B-015。ルート: 委任
- UI shell のネイティブ経路での失敗理由の保持: `src-tauri/src/adaptor/gateway/desktop_client.rs:97-104` の `DesktopClient::request` は `ConnectError` を `to_string()` で文字列にする。connectrpc 0.9.0 の `Display` は code と message だけを出して details を読まない。一方 `src-tauri/src/adaptor/controller/api/protocol/connect.rs` は業務理由を CommandError detail へ入れ、外側 message を「Command failed」に固定する。このため `src-tauri/src/adaptor/gateway/daemon_supervision.rs` の `request_shutdown` と `src-tauri/src/adaptor/gateway/login_item.rs` の `DaemonLoginPreference` が受け取る失敗理由は固定文になり、`DaemonBoundary` と設定画面にもその固定文が表示される。この状態を、サーバが返した具体的な失敗理由が表示される形へ変える。根拠: Thread `0b5bf893-ac5c-4840-a73d-a99fe52f9d18`、R-013 / B-015、R-011「desktop の機能は、変更前と同じ結果が画面へ反映される。」/ B-001。ルート: 委任
- renderer の長寿命 server stream と unary RPC の接続構成: renderer が接続する endpoint は平文の `http://127.0.0.1:{port}`（`src-tauri/src/adaptor/gateway/local_api.rs:280`）である。`src/lib/client.ts:59-84` は同一 baseUrl へ WebView の fetch で接続する。`SubscribePush`（`src/lib/client.ts:152-155`）と `AttachTerminalSurface`（`src/lib/client.ts:326-335`）は `timeoutMs: 0` の期限なし stream である。`src/screens/MainLayout.tsx:433` の `MAX_MOUNTED_PANES = 5` により、pane ごとの terminal stream（`src/hooks/useTerminal.ts:528`）が保持され、`AgentSessionPanel` の terminal も加わる。ブラウザは平文で HTTP/2 を使わない。そのため、長寿命 stream が同一ホストの HTTP/1.1 接続枠を占めると、入力・ack・状態取得の unary が接続待ちになる。ack を待つ出力 credit も解放されない。この状態を、保持可能な最大数の pane の terminal と push の stream が同時に開いていても、各 terminal への入力が届き、出力が継続して表示され、状態の再取得を含む unary RPC が接続待ちで止まらない形へ変える。固定するルート 5・6 と R-001 は維持する。根拠: Thread `eb315d10-ffa7-4ef4-948c-f8b82f22426b`、R-011「terminal の入出力と、backend 状態の push による画面更新が動作する。」/ B-004 / B-005。ルート: 委任

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
- daemon が実際に応答しなくなった場合に接続断として扱う、変更前と同じ監督の振る舞い。理由: 今周で変える対象は、要求単位の上限拒否を接続断と区別することに限られるため。
- 固定するルート 5（push = server streaming）・6（terminal 出力 = server streaming、入力 = unary）と R-001（desktop が backend を呼ぶ通信は Tauri IPC を経由しない）。理由: 接続構成の変更は、これらを保ったまま行うため。
- Requirements の Non-goals と design-02 の「変えないもの」に挙げた条件。design-02 の当該節をそのまま維持する。理由: 今周の決定は、いずれもこれらの範囲を変えないため。

## 未確定・リスク

- renderer の接続構成の変更について、同一ホストの接続枠の厳密な数と、接続待ちが発生する pane 数は未計測である（Thread `eb315d10-ffa7-4ef4-948c-f8b82f22426b` の指摘時点で、アプリを起動した再現と macOS の WebView での計測は未実施）。接続枠の数を前提とする構成を選び、その前提が外れると、R-011 / B-004 / B-005 を満たせない可能性がある。
- この周で Requirements・Behavior に自動判断による修正は加えていない。Assumptions に「自動判断: 未決」として残した要求、`[DEFERRED]` で人間へ渡した件、不成立として扱った件はいずれも無い。
