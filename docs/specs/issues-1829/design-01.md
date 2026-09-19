# Design 01

## 開始状態

- 初回。既存の `design-NN.md` は無く、開始状態の挙動は `docs/specs/issues-1829/requirements.md` の Current Behavior を参照する。
- 差分の基準は base ブランチ `main`、派生点 `00b57d77`。作業ブランチ `feat/issues/1829` は `020b097c` で、`6b3a9e27` / `a5c36a65` / `020b097c` の 3 コミットを派生点の上に持つ。
- 作業ツリーに未コミットの実装変更はない。untracked は `docs/specs/issues-1829/` の spec 文書だけである。
- この周までに解消・見送りとなった Thread はない。open Thread は無い。

## 変える部分

- クライアント向け RPC のサーバ実装: WebSocket route `/v1/client` と `Envelope` の自前ディスパッチを、Connect の service 実装へ置き換える。根拠: R-001「desktop を含むクライアントは、Connect プロトコルでサーバの RPC を呼ぶ」、B-001。ルート: `connectrpc` crate を使う（固定するルート 1）。
- `proto/client.proto` の定義: 現在 service を持たず `CommandRequest` / `Push` / `Envelope` の oneof だけを定義している状態から、Connect の service と method として定義する。根拠: R-001、R-005「生成したクライアントコードと transport アダプタだけで、desktop と同じ RPC を同じ手順で呼べる」、B-001、B-007。ルート: 委任
- Web クライアントのトランスポート: 自前 WebSocket クライアントを Connect の transport へ置き換える。根拠: R-001、R-005、B-001、B-007。ルート: `@connectrpc/connect-web` を使う（固定するルート 2）。
- 自前 RPC 層の削除: `src/lib/clientSocket.ts` と `src/generated/client_transport.ts` を削除する。根拠: R-004「`proto/client.proto` から生成されないクライアント側の RPC 層が存在しない」、B-006。ルート: この 2 ファイルを削除（固定するルート 3）。
- desktop の通信経路: Tauri IPC 中継（`DesktopClientSocket`、`attach_desktop_client` / `send_desktop_client_frame` / `detach_desktop_client`、UI shell Rust から daemon への WebSocket 中継）を廃止し、renderer が直接サーバの RPC を呼ぶ。根拠: R-001「desktop が backend を呼ぶ通信は Tauri IPC を経由しない」、B-002。ルート: 廃止する経路と Tauri command を指定（固定するルート 4）。
- backend 状態の push: `Envelope` の `push` / `push_resync` から Connect の server streaming へ移す。根拠: R-002、B-003。ルート: server streaming に載せることだけ固定（固定するルート 5）。1 本にするか種別ごとに分けるかは委任。
- terminal: `Envelope` の `stream` として同一接続に載る現在の形から、出力 = server streaming、入力 = unary へ分解する。根拠: R-003、B-004、B-005。ルート: RPC 種別だけ固定（固定するルート 6）。入出力の対応付けと、現行のフロー制御（`attachment_id` ＋ `sequence` / `ack`）を残すかどうかは委任。
- リクエスト単位の token 検証: handshake で 1 回だけ認証する現在の形から、要求ごとの検証へ変える。根拠: R-006「token を失効させた後に送られる次の要求は、接続の確立時点で有効だった場合でも拒否される」、B-008。ルート: 委任
- Origin 検証: Origin header を検証する処理が無い現在の状態から、クライアント向け RPC の Origin を検証する形へ変える。根拠: R-007、B-009、B-010。ルート: 委任。CORS preflight の扱いを含む。
- renderer の認証情報の取り扱い: renderer が token を一切持たず UI shell Rust が `client-api.json` を読んで接続する現在の形から、renderer 自身が RPC を呼ぶ形へ変わるため、master token を renderer へ出さないまま認証する経路を置く。根拠: R-008、B-011。ルート: 委任（`createConnectTransport` の `fetch` 差し替えと interceptor）。
- 接続断からの回復: 自前 RPC 層が持つ再接続・watch 再登録・stream ack の実装を、Connect 上の再接続と状態の再取得へ置き換える。根拠: R-010「接続の回復後に、状態の再取得、push の購読の復旧、terminal の再同期が行われ、現在の状態が画面へ反映される」、B-013。ルート: 委任
- 起動・切替中の変更要求の受付制御: `admit_client_command`、`DaemonSupervisionUsecase::admit_client_command`、`Supervision::client_command_admitted` を廃止する。根拠: R-016、B-018。ルート: 廃止対象を指定（固定するルート 7）。
- 結果を確認できない変更要求を扱う仕組み: クライアント側の状態機械（`operationQuery`、fingerprint による重複防止、未送信の再送）と、サーバ側の `src-tauri/src/domain/client_operation/`、`src-tauri/src/usecase/client_operation.rs`、`src-tauri/src/adaptor/controller/api/client_operation.rs` を廃止する。根拠: R-017「応答が返らなかった変更要求は、自動で再送されない。その要求の結果を照会する仕組みと、結果の受け取りが確認されていない変更要求の記録を保持する仕組みは存在しない」、B-019。ルート: 削除対象を指定（固定するルート 8）。
- `ClientConnectionBanner` の撤去: `src/components/ClientConnectionBanner.tsx` と `src/App.tsx:321` の描画を無くす。根拠: R-009、B-012。ルート: 撤去対象を指定（固定するルート 9）。
- 各 UI の通信状態表現の除去: `onUncertain` / `retryClientOperation` / `dismissClientOperation` を除去する。根拠: R-009、R-017、B-012、B-019。ルート: 除去対象を列挙（固定するルート 10）。
- milestone 77 の判断③・判断⑧: Connect を採る内容へ改訂する。根拠: R-014、B-016。ルート: 改訂先は GitHub milestone 77 の説明文だけ（固定するルート 11）。

## 固定するルート

1. Rust サーバのトランスポートに `connectrpc` crate を使う。範囲: クライアント向け RPC のサーバ実装。粒度: 使用する crate の指定のみで、配置と構成は委任。理由: local API が既に axum であり Tower / Axum 統合の上へ載せられ、1 サーバで Connect / gRPC / gRPC-Web を話せるため。
2. Web クライアントのトランスポートに `@connectrpc/connect-web` を使う。範囲: renderer のクライアント実装。粒度: 使用するパッケージの指定のみ。理由: トランスポート層の自前実装を無くし、他プラットフォームのクライアントが同じ手順でサーバを呼べる状態にするため。
3. 自前 RPC 層として `src/lib/clientSocket.ts` と `src/generated/client_transport.ts` を削除する。範囲: この 2 ファイル。粒度: 削除対象ファイルの指定。理由: proto から生成されない層であり、クライアントを増やすたびに各言語で再実装することになるため。
4. desktop の Tauri IPC 通信経路を廃止する。対象は `src/lib/desktopClientSocket.ts` の `DesktopClientSocket` と、`attach_desktop_client` / `send_desktop_client_frame` / `detach_desktop_client` / `admit_client_command`。範囲: renderer と daemon の間の通信経路。粒度: 廃止する経路と Tauri command の指定。理由: 他プラットフォームのクライアントで再現できない経路であり、desktop も他クライアントと同じプロトコルでサーバを呼ぶ状態にするため。
5. backend 状態の push を Connect の server streaming に載せる。範囲: クライアントへの push 配信。粒度: トランスポート方式の指定のみ。理由: 標準的な通信基盤へ寄せるため。
6. terminal を出力 = server streaming、入力 = unary に分解する。範囲: terminal の入出力。粒度: RPC 種別の指定のみ。理由: ブラウザから bidi / client streaming が使えない（fetch がリクエストボディをストリーミングできない）ため。
7. 起動・切替中に変更要求を拒否する受付制御を廃止する。対象は `admit_client_command`、`DaemonSupervisionUsecase::admit_client_command`、`Supervision::client_command_admitted`。範囲: 変更要求の送信前の受付判定。粒度: 廃止対象の指定。理由: `DaemonBoundary` が phase ready 以外で画面全体を覆い子を inert にするため利用者から見える振る舞いを作っておらず、残すと daemon が知らない UI shell の状態を Connect の経路へ持ち込むことになるため。
8. 結果を確認できない変更要求を扱う仕組みを、クライアント側・サーバ側ともに廃止する。サーバ側の削除対象は `src-tauri/src/domain/client_operation/`、`src-tauri/src/usecase/client_operation.rs`、`src-tauri/src/adaptor/controller/api/client_operation.rs`。範囲: 結果不明の変更要求の識別・重複判定・結果保持と、その自動再送。粒度: 削除対象の指定。理由: サーバ側の重複防止が必要になるのはクライアントが自動再送するからであり、利用者への提示を廃止すると支える側だけが残るため。
9. `ClientConnectionBanner` を廃止する。範囲: `src/components/ClientConnectionBanner.tsx` と `src/App.tsx:321` の描画。粒度: 撤去対象の指定。理由: 通信の内部状態（未送信・結果不明・再接続中）を利用者へ提示しないため。
10. 各 UI の `onUncertain` / `retryClientOperation` を除去する。範囲: `onUncertain` を渡す `CreateWorktreeModal` / `WorkspaceList` / `DeleteWorktreeDialog` / `SettingsModal` / `NodeContentView` / `useNotionSettings` / `useProviderAvailabilitySettings` / `useAppSettings` と、`retryClientOperation` を呼ぶ `ClientConnectionBanner` / `ProviderAvailabilitySettings` / `SettingsModal` / `NodeContentView`、および `ClientConnectionBanner` だけが呼ぶ `dismissClientOperation`。粒度: 除去対象の指定。理由: 結果不明を利用者へ提示して再試行させる経路を無くすため。
11. 判断③・判断⑧の改訂先は、判断の正本である GitHub milestone 77 の説明文だけとする。範囲: milestone 説明文。粒度: 改訂先の指定。理由: 判断を転記している既存 spec を改訂対象にしないため。

## 変えないもの

- daemon の起動・監視・停止とライフサイクル統合（Issue #1203）。`get_daemon_status` / `retry_daemon` / `quit_desktop` / `complete_desktop_restoration` / `fail_desktop_restoration` と `DaemonBoundary` の画面を含み、これらは Tauri command のまま残す。R-001 の「Tauri IPC を経由しない」が対象とするのは backend を呼ぶ通信であり、UI shell の Tauri command はその対象ではない。起動・切替中の受付制御だけは廃止対象とする。理由: #1203 の成果を変更しないため。
- CLI と provider hook が使う HTTP local API（workflow / provider-lifecycle）の入口と、その認証。リクエスト単位の token 検証と Origin 検証を加える範囲は、クライアント向け RPC の入口に限る。理由: 本変更の対象外であり、既存の入口の振る舞いを変えないため。
- UI shell に残す Tauri の機能（dialog、opener、updater ＋ process relaunch、autostart、`native-file-drop`、`menu-event`、menu / tray / window lifecycle）と、起動結果の取得・起動失敗時の処理。理由: 通信基盤の置き換えと独立しているため。
- 既存 spec（`docs/specs/issues-1199/`、`docs/specs/issues-1200/`、`docs/specs/issues-1201/`、`docs/specs/issues-1202/`、`docs/specs/issues-1203/`）。判断③・判断⑧を転記した箇所と `docs/specs/issues-1201/behavior.md` の B-009〜B-031 を含め、一切変更しない。理由: 終わった spec を改訂対象にしないため。
- milestone 77 の判断①（`.proto` が protocol の正）、判断④（ローカルは loopback ＋ token ファイル認証）、判断⑦（master token を renderer へ露出させない）。理由: 本変更が見直すのは判断③と判断⑧だけであるため。
- 対象はローカル loopback 上の接続だけとし、リモートアクセス経路は扱わない。理由: 本変更の対象外であるため。

## 未確定・リスク

- restoration の完了経路が、削除対象の通信経路に依存している。`src/components/DaemonBoundary.tsx:89` が `completeClientRestoration`（`src/lib/clientSocket.ts:1046`）を呼び、`DesktopClientSocket.completeRestoration` を経て `complete_desktop_restoration(launch_id, attachment_id, generation)` へ渡る。サーバ側でも restoration の開始と完了は desktop attachment に結び付いている（`DaemonSupervisionUsecase::attach` が `begin_restoration` を行い、`finish_restoration` が `attachment_id` を取る。`src-tauri/src/usecase/daemon_supervision.rs:231`、`:240`）。削除対象の `clientSocket.ts` と `DesktopClientSocket` がこの経路の唯一の呼び出し元であり、`attachment_id` の供給元でもある。「#1203 のライフサイクル統合は変更しない」と両立させる方法が未確定で、想定が外れると R-013 / B-015 を満たせない。
- desktop の WebView から daemon（`http://127.0.0.1:<port>`）への Connect 要求が、WebView の origin（macOS / Linux は `tauri://localhost`、Windows は `http://tauri.localhost`）とスキームの異なる相手への fetch として成立するかが未確認。成立しない場合、R-001 / B-001 / B-010 を満たせない。
- `connectrpc` crate は pre-1.0 であり、server streaming を含む必要な機能が揃っているかが未確認。揃っていない場合、R-002 / R-003 と B-003 / B-004 を満たせない。リスク受容の判断は委任されている。
- この周で Requirements / Behavior に自動判断による修正は加えていない。Assumptions に「自動判断: 未決」として残した要求はなく、`[DEFERRED]` で人間へ渡した件もない。
