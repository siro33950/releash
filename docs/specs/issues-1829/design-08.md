# Design 08

## 開始状態

- 差分の基準は base ブランチ `feat/issues/1203`、派生点 `020b097c`（design-07 と同じ）。作業ブランチ `feat/issues/1829` の HEAD は `020b097c` で、この変更はすべて作業ツリーの未コミット変更として存在する。
- 直前の Design は `docs/specs/issues-1829/design-07.md`。その「変える部分」1 件は実装済みで、対応する Thread `eb315d10-ffa7-4ef4-948c-f8b82f22426b` は resolved となった。見送り（`[DEFERRED]`）とした Thread、不成立（`[REJECTED]`）とした Thread はいずれも 0 件である。
- 今周で変える部分は、`[FIX_POLICY]` を持つ open Thread 12 件である。
- Requirements・Behavior は今周で変更していない。R-001〜R-014 / R-016〜R-018 と B-001〜B-016 / B-018〜B-020 の対応は保たれている（R-015 / B-017 は欠番）。対応表は現存するすべての Requirement ID 17 件を網羅し、参照先の Behavior ID はすべて本文に存在する。自動判断による修正は加えていない。

## 変える部分

- terminal 購読の登録と受理判断の所有層: terminal 購読の登録簿、重複・上限・終了済みの受理判断、購読の登録と解放を controller から内側の層へ移し、controller は入口として内側の層へ処理を渡すだけにする。開始状態では、これらを `src-tauri/src/adaptor/controller/api/client_stream.rs` が持つ（`:16,22` の登録簿、`:37-83` の subscribe、`:85-122` の attach、`:174-178` の解放）。`SubscribeTerminalSurfaces` / `AttachTerminalSurface` の、外部から観測できる結果は変えない。重複・上限・終了済みの場合に返すエラーもこれに含む。根拠: Thread `2fa233d9-402e-45f9-8443-678d6a8dd92f`（`docs/architecture/CONTROLLER.md` の「Usecase を呼ぶだけ」「受理判定を controller で書かない」に反する。修正の採否は自動判断）、R-003「terminal の出力は Connect の server streaming で配信され、terminal への入力は unary の RPC で送られる。」/ B-004 / B-005。ルート: 委任
- 同じ attachment_id での再 attach: 同じ attachment_id で `AttachTerminalSurface` が成功した後に先行の attachment が終了・解放されても、成功した attachment の出力は配信され続け、入力も受け付けられるようにする。開始状態では、`src-tauri/src/adaptor/controller/api/client_stream.rs:85-122` の attach が同じ ID の重複を判定しない。旧 attachment を解放すると（`:186-190`）、同じ ID の新しい登録まで取り除かれる。renderer も、旧 stream の Closed を受けると同じ ID の listener を削除する（`src/lib/client.ts:360-362`）。根拠: Thread `d5f17122-36a6-4778-9a77-85cf86e4a11a`（派生点 `020b097c` からの回帰）、R-011「terminal の入出力と、backend 状態の push による画面更新が動作する。」/ B-004 / B-005。ルート: 委任
- 終了済み terminal の stream 完了の扱いを判断する層: 終了済み terminal の stream が完了しても再同期を始めない、という判断を Rust の側に置き、renderer は受け取った結果に従うだけにする。開始状態では、renderer（`src/lib/client.ts:403,422-436`）が exit または `snapshot.is_exited` から終了済みかを判定し、`onClosed`（`src/hooks/useTerminal.ts:530-535` の再アタッチ）を呼ぶかを決めている。サーバ（`src-tauri/src/adaptor/controller/api/client_stream.rs:113-121`）は終了理由を区別せずに Closed を送る。外部から観測できる結果は変えない。終了済み terminal では再同期せず、終了していない terminal の stream が終了した場合は再同期する。根拠: Thread `a2d54c39-8af4-4cda-bb19-d14c4e8aa4ad`（AGENTS.md「全てのアプリケーションロジックは Rust に置く。例外なし。」に反する。修正の採否は自動判断）、R-010「接続の回復後に、状態の再取得、push の購読の復旧、terminal の再同期が行われ」/ B-013、R-011 / B-004。ルート: 委任
- attachment 解放時の `DetachTerminalSurface` の送信: 通常解放・世代不一致・再同期のどの場合も、一つの attachment の解放につき `DetachTerminalSurface` を一度だけ送り、送信失敗の扱いを一通りに定める。開始状態では、同じ attachment ID へ 2 回送っている。1 回目は `attachClientStream` の release（`src/lib/client.ts:393-399`）、2 回目は `src/hooks/useTerminal.ts` の `detach_terminal_surface`（`:272-276`、`:543-546`、`:558-562`）である。根拠: Thread `e868111a-88d7-440a-8d49-c55ba135c348`、R-003 / R-011 / B-004 / B-005。ルート: 委任
- 通常の push 配信で行う往復変換: 通常の push 配信で、受信済みの符号化 payload を扱う往復変換をなくす。開始状態では、購読ごとに payload を prost で復号・再符号化してから配信形式へ変換している（`src-tauri/src/adaptor/controller/api/client_service.rs:39-41,58` から `src-tauri/src/adaptor/protocol/connect.rs:11-16`）。配信する push の内容と、初回と Lagged 時の Resync の配信は変えない。根拠: Thread `84a9f975-88db-40a3-98ce-b4193162e973`（AGENTS.md「full-retention 設計を避ける。… clone / store / recompute / resend しない。」に反する）、R-002「backend 状態の push は、Connect の server streaming でクライアントへ配信され、画面へ反映される。」/ B-003。ルート: 委任
- watcher の生成・停止と push 購読受付のロック: ブロックしうる watcher の生成・停止を、共通の購読ロックを持ったまま実行しないようにする。開始状態では `src-tauri/src/usecase/watcher.rs:57-60,86-100` がこのロックを持ったまま実行している。変更後は、その実行中も別の worktree の監視開始と push の購読受付が待たされず、async の handler（`src-tauri/src/adaptor/controller/api/client_service.rs:30` の `subscribe_push`）もこの処理を待って tokio の worker を止めない。購読数の上限と、購読ごとに watcher を所有する条件は変えない。根拠: Thread `d0bfee7d-d6b1-481e-93ac-9738ef612641`、R-002 / B-003、R-010「… push の購読の復旧 … が行われ」/ B-013。ルート: 委任
- desktop の復元状態を持つ層: desktop の復元状態と復元の受理規則を domain の集約に持たせ、gateway は自前の可変状態で復元の受理を判定したり遷移させたりしないようにする。復元状態とは、attachment が現在のものか、復元が完了したか、を指す。開始状態では、`src-tauri/src/adaptor/gateway/desktop_client.rs:34-35,86-99` が attachment ID と restored を持ち、受理の判定と遷移を行っている。さらに `src-tauri/src/usecase/daemon_supervision.rs:206-237` が domain の判断と gateway の判断を組み合わせている。復元の成否と `DaemonBoundary` の画面の振る舞いは変えない。根拠: Thread `3c7a703a-1ba5-4e74-ac6c-4f837a0c5b17`（`docs/architecture/GATEWAY.md`「gateway は状態機械を持たない」に反する。修正の採否は自動判断）、R-013「daemon の起動・監視・停止と `DaemonBoundary` の画面は、変更前と同じ振る舞いである。」/ B-015。ルート: 委任
- `AbortSignal.any` を持たない WebKit での RPC 送信: 支持する macOS（`src-tauri/tauri.conf.json:62` の `minimumSystemVersion` 13.0）の範囲には、`AbortSignal.any` を持たない WebKit の WebView も含まれる。そうした WebView でも、初回 `GetServerInfo` を含む RPC が送信前に失敗せず、接続を確立して既存の画面操作ができるようにする。開始状態では、`src/lib/client.ts:79-84` の transport の fetch が `AbortSignal.any` を無条件に呼んでいる。支持する macOS の範囲と、接続を中断（abort）したときの効果は変えない。根拠: Thread `b2f4ee2d-b48f-4297-b774-e6383acbb8d8`、R-001「desktop を含むクライアントは、Connect プロトコルでサーバの RPC を呼ぶ。」、R-011「desktop の機能は、変更前と同じ結果が画面へ反映される。」/ B-001。ルート: 委任
- usecase テストから adaptor への逆依存: usecase のテストが adaptor 層の型・定数を参照しないようにする。Tauri command の登録内容を検証する場合は、登録を持つ層のテストに置く。開始状態では、`src-tauri/src/usecase/daemon_supervision_test.rs:324-338` が `crate::adaptor::controller::command::client::COMMAND_NAMES` を参照している。根拠: Thread `d434c5e4-c62a-4aa0-9a1a-f864b924204c`（`docs/architecture/README.md` の逆依存の禁止）、R-016 / B-018。ルート: 委任
- `connection_pending` の境界の domain テスト: `connection_pending` が Stopped で false、Backoff / Stopping / Installing で true であることを domain テストで検証する。これにより、Stopped を待機対象に含める変更や、切替中の phase で待機を終える変更をテストが検出できるようにする。開始状態の `src-tauri/src/domain/daemon_supervision_test.rs:441-450` は、初期状態・connected の後・fail_restoration の後の 3 点しか見ていない。根拠: Thread `49feb5aa-ce10-43b9-9994-cd1eecafc470`（`docs/architecture/TEST.md` で domain のテストは必須）、R-016「クライアントの変更要求は、daemon が起動中または切替中であることを理由に送信前に拒否されない。」/ B-018。ルート: 委任
- 切断後の push 購読枠の解放を確かめる統合テスト: 切断後の再購読を検証するとき、購読枠が解放されたことを期限付きで確かめてから、16 購読が成功するかを判定する。開始状態の `src-tauri/tests/client_api/mod.rs:454-458` は、固定 100 ms 待つだけで解放されたとみなしている。変更後は、解放されない不具合があればテストが失敗し、実行が遅いだけでは失敗しない。根拠: Thread `78515371-ff1b-4b95-b78b-4f1124595ad0`、R-002 / B-003、R-010 / B-013。ルート: 委任
- 購読の途中で届く resync の renderer テスト: 確立済みの push 購読の途中で resync を受けた場合を renderer のテストで検証する。確かめる内容は、`listenClient` の `onReconnect` と `onClientRefresh` の listener が呼ばれ、その後の push が listener に届くことである。途中の resync を無視する変更や、`onReconnect` を通知しない変更をすると、このテストが失敗するようにする。開始状態の `src/lib/client.test.ts` は、購読の先頭で届く resync しか扱っておらず、`listenClient` に第 3 引数も渡していない。根拠: Thread `bb312067-a1e3-483b-8dc9-20a525bfff95`、R-002 / B-003、R-010 / B-013「THEN 状態の再取得、push の購読の復旧、terminal の再同期が行われる AND 画面には現在の状態が反映される」。ルート: 委任

## 固定するルート

今周で新たに固定する実装上の指定はない。design-01 で固定したルート 1〜11 を、design-07 に続いて今周も維持する。解除するルートはない。

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
- 固定するルート 5（push = server streaming）・6（terminal の出力 = server streaming、入力 = unary）と、R-001（desktop が backend を呼ぶ通信は Tauri IPC を経由しない）。理由: 今周の terminal・push の変更（terminal 購読の所有層を含む）は、RPC 種別とトランスポートを保ったまま行うため。
- design-01〜design-07 の記載。理由: 差分の基準の変更は design-07 の開始状態に記載済みであり、過去の周の Design は書き換えないため。
- Requirements の Non-goals と、design-02 の「変えないもの」に挙げた条件。design-02 の当該節をそのまま維持する。理由: 今周の決定は、どれもこれらの範囲を変えないため。

## 未確定・リスク

- `AbortSignal.any` を持たない WebKit の実環境で動かした結果は、開始状態でも Thread でも確認されていない。Connect の呼出経路（`src/lib/client.ts`、`@connectrpc/connect`、`@connectrpc/connect-web`）では、`AbortSignal.any` 以外にその WebKit に無い API を使っていない。これはコード検索で確かめたもので、実行して確かめたものではない。この前提が外れると、R-001 / R-011 / B-001 を満たせない可能性がある。
- この周で Requirements・Behavior に自動判断による修正は加えていない。Assumptions に「自動判断: 未決」として残した要求、`[DEFERRED]` で人間へ渡した件、不成立として扱った件はいずれも無い。
- 分類 design の Thread 3 件（`2fa233d9-402e-45f9-8443-678d6a8dd92f` / `a2d54c39-8af4-4cda-bb19-d14c4e8aa4ad` / `3c7a703a-1ba5-4e74-ac6c-4f837a0c5b17`）は、既存の規約に反する事実を示しているため、修正するかどうかを自動で判断した。人間による採否の確認は経ていない。
