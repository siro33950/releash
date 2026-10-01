# Design

## 変える部分
- 画面の作り直しの削除: `src/components/DaemonBoundary.tsx:124-135` の key（`connectionGeneration`）と、シェルの phase が `ready`・`restoring` 以外で画面を描かない条件を削除する。一度シェルの phase が Ready になって画面を出した後は、phase にかかわらず画面を描き続ける。根拠: R-001「一度出した画面は…作り直されず、unmount されない」、B-001。ルート: 委任
- 「再接続中」の表示: `DaemonBoundary` が `src/lib/client.ts` の接続状態（`getConnectionState`・`onConnectionStateChange`）を読み、一度 READY になった後に READY でなくなったら、画面を残したまま「再接続中」を出す。根拠: R-002、B-002〜B-004。ルート: 画面の接続状態だけを条件にし、シェルの phase は条件にしない。表示の形は委任
- 覆いの出し分け: `DaemonBoundary.tsx:136-178` の覆いを、一度画面を出した後は Failed・Stopping・Installing・Stopped のときだけ出し、Starting・Backoff では出さない。最初の起動（一度も Ready になっていない間）は今の覆いのまま。根拠: R-003〜R-005、B-004〜B-006。ルート: 委任
- 復元の手続きの削除（画面側）: `DaemonBoundary.tsx` の `RestorationContext`・`useDesktopRestoration`・`complete`・`fail`、`src/lib/client.ts` の `completeClientRestoration`・`restorationAttachmentId`・`Session.attachmentId`、`src/App.tsx:105-115` の復元の完了と失敗の呼び出しを削除する。`get_client_endpoint` に attachment ID を渡さない。根拠: R-007、B-008。ルート: 委任
- 復元の手続きの削除（シェル側）: `src-tauri/src/adaptor/controller/command/client.rs` の `complete_desktop_restoration`・`fail_desktop_restoration`、`get_client_endpoint` の `attachment_id`、`src-tauri/src/usecase/daemon_supervision.rs` の `finish_restoration`・`fail_restoration`・`attach` の `begin_restoration`・`DaemonStatus.connection_generation`・Retry の `retry_restoration` の分岐、`src-tauri/src/domain/daemon_supervision.rs` の Restoring の phase・`connection_generation`・`restoration_deadline`・`attachment_id`・`begin_restoration`・`finish_restoration`・`fail_restoration`・`expire_restoration`・`retry_restoration`・`restoration_current`・`restoration_failed`・`FailureStage::Restoration`・`ShellOperation::RestoreState`、`src-tauri/src/adaptor/presenter/daemon_status.rs` の `connection_generation`、`command/mod.rs` の `shell_operation` の該当するコマンド名を削除する。`connected()` はつながったら Ready にする。根拠: R-007「シェルは、つながったら復元の完了を待たずに Ready になる」、B-008。ルート: 委任
- 起動時の処理と自動の更新の確認: `App.tsx:94-96` の `useUpdateChecker` の条件と、`App.tsx:148-171` の起動時の処理を、`restoration.ready` ではなく、画面を最初に出したときに 1 回だけ動く形にする。自動の更新の確認は `settings.autoUpdate` に合わせ、ON にしたときにも確認する。根拠: R-008、B-009、B-013。ルート: シェルの phase には合わせない。自動の更新の確認は `useUpdateChecker(settings.autoUpdate)` とし、確認が済んだ結果を `useUpdateChecker` が持って確認を繰り返さない。確認が失敗したら、持っている結果を空に戻す
- メニューの有効・無効: `App.tsx:252-256` の `set_menu_items_enabled` を、worktree の選択が変わったときに加えて、シェルの phase が Ready に戻ったときにも呼ぶ。根拠: R-009、B-010。ルート: 委任
- performance metrics の設定: 設定画面（`src/components/panels/SettingsModal.tsx:784-799`）で、`desktop-settings` を読めていない間は performance metrics の値を保存せず（`:933-935,954-956`）、読み込みの失敗（`src/hooks/useSettings.ts:71` の `loadError`）をその設定の場所に出す。daemon への書き込みの失敗もその設定の場所に出し、書き込みが済むまで変えた値を保存されていないものとして残す。根拠: R-010、B-011、B-012、B-015、B-016。ルート: 委任

## 固定するルート
- 「再接続中」の表示の条件は、`src/lib/client.ts` が持つ画面の接続状態だけにする。シェルの phase は条件にしない。理由: #1895 で、操作の受付の可否は画面の接続状態だけで決まる形にした。シェルの phase を条件にすると、画面の接続が READY で操作が通るのに「再接続中」と出て、表示と振る舞いが食い違う。
- 覆いは、接続が戻っても画面を使い続けられない phase（Failed・Stopping・Installing・Stopped）だけに出す。
- 起動時の処理と自動の更新の確認は、画面を最初に出したときに 1 回だけ動かし、シェルの phase には合わせない。自動の更新の確認は、設定で ON にしたときにも動かす。理由: つなぎ直すたびに動かすと、cwd の Repository の登録とタブを開く処理を繰り返し、利用者が閉じたタブも開き直す。画面を最初に出す時点ではシェルの phase は Ready なので、`check_desktop_update` は拒否されない。
- 設定の読み込みの失敗は、`desktop-settings` の値を使う場所に出す。`restoration.fail` への受け渡しは、復元の手続きと一緒に消す。

## 変えないもの
- `DaemonBoundary` がシェルの状態を受け取る方法（`subscribe_daemon_status` による変化の通知）。#1895 で変えたもので、#1896 では変えないと決めたため。
- 利用者の操作によるシェルの Tauri コマンドの呼び出し（`src/hooks/useAppSettings.ts:40,75,108,115`、`src/hooks/useUpdateChecker.ts:69`）。既に失敗を画面に出しているため。
- 設定画面の Background の欄の、読み込みの失敗の表示（`useAppSettings.ts:65,131` → `SettingsModal.tsx:747-750`）。既に出しているため。

## 未確定・リスク
- requirements の Current Behavior（復元が必ず失敗する流れを含む）は、コードを読んで確認したもので、実機での再現は行っていない。
