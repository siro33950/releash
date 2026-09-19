# Design 02

## 開始状態

- 差分の基準は `main` から派生した `feat/issues/1203` であり、派生点は `00b57d77`（`feat(daemon): Tauri非依存のバックエンドを抽出する (#1825)`）。開始状態は、派生点に Design 01 の実装（未コミットの 90 ファイルの差分）を加えた現行のワークツリーである。
- 直前の Design は `docs/specs/issues-1203/design-01.md`。Design 01 で実装済みの変更は本文書に再掲しない。
- この周までに解消・見送りとなった Thread は無い。resolve した Thread、`[DEFERRED]`、`[REJECTED]` はいずれも 0 件であり、`[FIX_POLICY]` を持つ open Thread が 23 件残っている。

## 変える部分

- 利用者の終了操作と OS 起因の終了の一本化: アプリケーションメニューの Quit、Cmd+Q、Dock アイコンの終了、AppleScript の quit、メニューバーのアイコンの Quit と、macOS のログアウト・再起動・システム終了のいずれでも、daemon への停止要求と一括停止を経てから UI と daemon が終了するようにする。開始状態では `menu.rs` の `.quit()` が `terminate:` を直接呼び、メニューバーのアイコンの Quit だけが停止要求へ届く。根拠: R-006「利用者の終了操作（アプリケーションメニューの Quit、Cmd+Q、Dock アイコンの終了、メニューバーのアイコンの Quit、AppleScript の quit）を行うと、UI が daemon へ停止要求を送り、daemon の一括停止……のうち利用者の判断を必要としない段階が完了してから、UI と daemon の両方が終了する。一括停止が有限の期限内に完了しない場合も、UI と daemon は終了する」、R-029「macOS のログアウト、再起動、システム終了による終了でも、R-006 と同じ停止が行われ……この停止はログアウト、再起動、システム終了を妨げない」、B-006 / B-033。ルート: 委任（期限の具体値も委任）
- ログイン項目が承認を必要とする状態の扱い: 承認待ちの状態を「無効」として示し、承認が必要であることと承認を行う場所への導線を出し、この状態で UI の起動を失敗させないようにする。開始状態では `login_item.rs` の `status == 2` が `Err` になり、`refresh_registration` 経由で `desktop.rs` の `setup` が失敗する。根拠: R-030「ログイン時に起動の登録が macOS の承認を必要とする状態のとき、ログイン時に起動の設定は無効として示され、承認が必要であることと承認を行う場所への導線が示される。この状態でも UI の起動は失敗しない」、B-034 / B-035、Thread 6223510a。ルート: requiresApproval を「無効」として扱い、承認が必要な旨と System Settings への導線を表示する。起動経路はこの状態で失敗させない（「固定するルート」）
- daemon の起動完了を待つ間のウィンドウの表示: cold start で daemon の起動完了を待つ間もウィンドウを出し、起動中であることを示すようにする。開始状態では `observe_with` が `failed` / `backoff` / `stopping` と ready でしかウィンドウを出さず、`starting` の間は何も表示されない。根拠: R-031「`Releash.app` を起動すると、daemon の起動完了を待つ間もウィンドウが表示され、起動中であることが示される。最小化状態で起動の設定が有効なログイン時の起動では、このウィンドウも表示されない」、B-036 / B-037。ルート: 委任
- ログイン項目の登録が失われた場合の再登録: ログイン時に起動を有効にしていた状態で OS の登録が失われていたら、UI の起動時に現在の場所で登録し直し、設定を有効のまま保つ。開始状態の `refresh_registration` は後継 UI の起動時にだけ呼ばれ、`native::enabled()` が真のときしか登録し直さないため、登録が失われた状態（status が notFound）では何もしない。根拠: R-032「ログイン時に起動を有効にしていた状態で OS のログイン項目の登録が失われた場合、UI の起動時に現在の場所で登録し直され、ログイン時に起動の設定は有効のままになる」、B-038。ルート: 委任
- 読み取り専用の一時的な場所での登録の拒否: `Releash.app` が読み取り専用の一時的な場所で実行されているとき、ログイン時に起動の登録を行わず、登録できない理由を示すようにする。開始状態にこの判定は `cli_install.rs` にしかなく、ログイン項目の登録経路は判定しない。根拠: R-033「`Releash.app` が読み取り専用の一時的な場所で実行されている場合、ログイン時に起動の登録は行われず、登録できない理由が示される」、B-039。ルート: 委任（判定手段も委任）
- `/usr/local/bin/releash` を設置する契機: 設置を利用者の明示操作だけで行い、UI と daemon の起動時には設置せず、起動時に管理者の認証を求めないようにする。開始状態では `lib.rs` が daemon の起動時に `ensure_cli_symlink_installed` を呼び、直接 symlink を作れない場合に管理者の認証を求める。根拠: R-034「`/usr/local/bin/releash` の設置は、利用者が設定で明示的に操作したときにだけ行われる。UI または daemon の起動時には設置されず、起動時に管理者の認証を求められない。設置に管理者の認証が必要な場合は、その明示操作の中で求められる」、B-040 / B-041。ルート: 委任
- メニューバーのアイコンの表示: ライトとダークのメニューバー、メニューバーの色付け、アイコンの選択状態に追従して表示されるようにする。開始状態の `tray.rs` はカラーの `icons/32x32.png` をそのまま使い、template 画像の指定が無い。根拠: R-035「メニューバーのアイコンは、ライトとダークのメニューバー、メニューバーの色付け、アイコンの選択状態に追従して表示される」、B-042。ルート: 委任（素材の作成も委任）
- 結果不明の記録の終端: 再起動または切替をまたいで残った結果不明の記録を、利用者が確認したときにだけ破棄し、破棄した後は同じ対象への変更要求を通常どおり受け付けるようにする。自動では破棄しない。開始状態では `clientSocket.ts` が復元した参照を pending へ戻さず、除去が pending の応答処理に限られるため、終端が無い。根拠: R-027「結果不明の記録は、利用者がその結果不明を確認したときにだけ破棄され、破棄した後は同じ対象への変更要求が通常どおり受け付けられる。利用者が確認するまで、この記録は自動で破棄されない」、B-031 / B-043、Thread 57af43e5。ルート: 委任
- 旧 autostart 登録からの移行経路の削除: 旧 `tauri-plugin-autostart` の登録を検出して新しい登録方式へ移す経路（`legacy_registered` / `migrate_legacy` と、それを使う `desktop.rs` の後継起動と `setup` の呼び出し）を無くす。開始状態にはこの移行がある。根拠: Requirements の Scope / Non-goals（変更しないもの）へ「旧 `tauri-plugin-autostart` の登録から新しい登録方式への移行」が加わり、本 Issue の対象から外れたこと。ルート: 後方互換のための機能を追加せず、既存の後方互換経路も残さない（「固定するルート」）
- クライアント接続の単一経路への集約: 監督用に別途張る ws 接続と要求追跡を、desktop のクライアント接続の単一経路へまとめ、Quit と更新でも接続が増えないようにする。根拠: Thread e3ec1f23（`daemon_supervision.rs` の独自 `connect`、`request_shutdown`、`connection()` が通常操作とは別の接続を張る。受入条件は接続と要求追跡が単一経路へ集約され、Quit と更新でも接続が増えないこと）。ルート: 委任
- platform infrastructure の依存方向の是正: `tray` と `window_lifecycle` が controller / usecase を直接呼び、controller が `tray` を呼ぶ相互参照を無くし、infrastructure が内側の層を知らない向きで同じ操作が成立するようにする。根拠: Thread e3890e32（受入条件は platform の infrastructure が controller / usecase を知らない依存方向で、tray と window の表示・終了操作が同じように成立すること）。ルート: 委任
- 状態の再取得が完了するまでの操作受付の停止: 接続の検証と必要な状態の再取得が完了するまで、通常の操作を受理せずキューにも入れないようにする。根拠: Thread e047d2e8、R-023「切替の間、UI は通常の操作を受け付けない。新 daemon が Ready に達し、接続先とリリースの一致と認証済みのクライアント ws 接続を確認し、必要な状態を再取得した後に受付を再開する」、B-025 / B-030。ルート: 委任
- spawn 済みで未接続の状態の停止要求の検証: 終了確認の成功時と終了未確認時の結果、および再 spawn を行わないことを usecase の実行経路で検証する。根拠: Thread bdc01054（受入条件は同分岐の結果と再 spawn を行わないことが usecase の実行経路で検証されること）。ルート: 委任
- 停止要求の応答の扱い: 正常な一括停止と終了確認の後に更新の適用が進み、かつ Accepted の応答だけで停止完了と誤認しないようにする。根拠: Thread b42e9b6e（`request_shutdown` に Ok を返す経路が無く、Accepted でも切断または期限で Err になる）、R-021 / R-024、B-025 / B-027。ルート: 委任
- 未送信と確定した変更要求の marker の扱い: 送信されていないと確定した要求が再起動後も未送信の分類を保ち、結果不明の先行要求として同一 fingerprint の要求や同じ ordering target の後続を拘束しないようにする。根拠: Thread a6a33aba（`clientSocket.ts` が remember の後に forget せず return し、次回起動で同じ ID が unknown として復元される）。ルート: 委任
- 接続の検証が済むまでの command の受理範囲: 接続先とリリースの確認が済むまで、復旧・状態確認・終了以外の通常 command を Rust の入口で拒否し、startup-failure の画面から OS 設定や永続データを変更できないようにする。根拠: Thread a38d4bda、R-002 / R-020、B-023 / B-024。ルート: 委任
- 更新 gateway の境界の検証: `TauriUpdateGateway` 相当の境界を通して、download / install / restart の結果と失敗理由の伝播を検証する。根拠: Thread 90d30273（`gateway/desktop_update.rs` に対応するテストが無い）、R-021 / R-025。ルート: 委任
- ログイン項目の状態の読み書きの一本化: ログイン時に起動を無効にした後、設定画面を開き直しても無効と表示され、System Settings の表示と一致し、`.app` の外に無効化のレコードが残らないようにする。根拠: Thread 7df60497（`disabled()` が `"com.releash.app" => true` と行全体を比較し、`set_enabled(false)` が launchctl disable だけで登録を残す）、R-008 / R-010 / R-011。ルート: `SMAppService` の `register()` / `unregister()` / `status()` だけを使い、`launchctl` の `enable` / `disable` / `print-disabled` を呼ぶ経路を削除する（「固定するルート」）
- 到達不能な Reopen フォールバックの削除: Reopen の表示経路を、到達可能な現行の起動構成に対応する 1 系統だけにする。根拠: Thread 6cd75e5e（`window_lifecycle.rs` の supervisor 不在時の分岐と `show_and_focus_active_window` が本番経路で到達不能）。ルート: 委任
- 復元した結果不明の項目に提示する操作: 提示される操作が実際に行える処理と一致し、照会できない操作を有効として提示しないようにする。根拠: Thread 6452a905（`retryClientOperation` が pending にある要求しか query せず、復元項目では照会が行われない）。ルート: 委任
- 新規 Rust テストの構造: 指摘された各テストで前提・操作・検証が Given / When / Then の区分に分かれ、複数の状態遷移を含むケースでも前提と期待値の対応が追えるようにする。根拠: Thread 5b0fe493（`domain/daemon_supervision_test.rs` ほか 5 ファイルに区分が無い）。ルート: 委任
- 起動期限超過で子の終了を確認できない場合の扱い: 起動監督の待機が有限に終わり、失敗の段階と理由および終了操作が表示され、終了確認前の再 spawn が行われないようにする。根拠: Thread 5446363d（`stop_failed` が Phase を Starting のまま残し、停止待ちが繰り返される）、R-015 / R-016、B-019。ルート: 委任
- 停止応答と利用者判断を挟む切替の検証: 停止要求が Accepted 以外で返る場合の表示と切替の抑止、および必要な利用者判断を経た後の終了確認を、usecase と gateway の連携境界で検証する。根拠: Thread 4213f7d2（`FakeDaemon::request_shutdown` が常に Ok を返す）、R-021 / R-024、B-025 / B-027。ルート: 委任
- 親終了時の子プロセスの回収と spawn の同期: 親終了時の回収と、daemon から到達可能な全ての子プロセス生成を同期させ、UI の終了後に子プロセスが取り残されないようにする。根拠: Thread 3db7306d（`git_host/github.rs` と `code/staging.rs` が spawn_guard の lock を取らずに直接 spawn する）、R-005、B-005。ルート: 委任
- handoff ファイルの保持件数の契約の検証: 上限ちょうど・新規 ID で上限を超えるとき・上限に達した状態で既存 ID を更新するとき・上限を超えたディレクトリを list するときの各契約を、実 gateway を通して検証する。根拠: Thread 392c08f1、R-027。ルート: 委任
- 停止要求なしの正常終了で再 spawn しないことの検証: 停止要求が無い状態で Ready 後の daemon が正常終了した場合に、backoff と再 spawn へ進まないことを usecase の実行経路で検証する。根拠: Thread 276f3707、R-018「daemon の正常終了と spawn の失敗では自動再起動を行わない」。ルート: 委任
- 起動・停止・切替の受理判断と失敗分類の所有: これらの判断と分類を domain の判断として実行経路に置き、表示用の phase 文字列を独立した判断の正本にしないようにする。根拠: Thread 1910f237（`wait_for_update_stop` と `controller/desktop_lifecycle.rs` の phase / stage 文字列判断）。ルート: 委任
- 更新の適用要求の排他の検証: 更新の適用中に第 2 の適用要求が拒否され、download / 停止 / install / restart が重複して行われないことを usecase の実行経路で検証する。根拠: Thread 120d37a6、R-022。ルート: 委任
- 設定 hook の中継禁止の検証: 必要なログイン項目の呼び出しを許しつつ、shell / window preferences を Tauri へ中継しないという条件を、同じ 4 ケースで検証する。根拠: Thread 096c2bfb（`not.toHaveBeenCalled()` が `toHaveBeenCalledWith` へ置換され否定条件が消えた）。ルート: 委任
- 再起動操作の実装の一本化: 更新経路と更新を伴わない Restart が、後継 UI の起動・Quit フラグの設定・自 UI の終了という同一の再起動操作の実装を共有し、手順の一致が構造的に保証されるようにする。根拠: Thread 02d36448（`gateway/desktop_update.rs` と `controller/desktop_lifecycle.rs` が同じ手順を二重に実装）、R-021 / R-026。ルート: 委任

## 固定するルート

- ログイン項目の操作は `SMAppService` の `register()` / `unregister()` / `status()` だけを使い、`launchctl` の `enable` / `disable` / `print-disabled` を呼ぶ経路を削除する。粒度は使う API と削除する経路の指定まで。理由は、Apple が示す対がこの 3 つであり、`launchctl` の `disable` は `.app` の外にある別のストアで再起動をまたいで永続するため。関係: R-008 / R-010 / R-011、Thread 7df60497。
- 承認を必要とする状態（requiresApproval）は「無効」として扱い、承認が必要な旨と System Settings への導線を表示する。起動経路はこの状態で失敗させない。粒度は状態の扱いと起動を失敗させないことの指定まで。関係: R-030、B-034 / B-035、Thread 6223510a。
- 後方互換のための機能を追加せず、既存の後方互換経路も残さない。粒度は経路を残さないという指定まで。関係: 旧 autostart 登録からの移行経路の削除。
- Design 01 で固定した次のルートは今周も維持する。LaunchAgent は `SMAppService` の agent 方式を使い plist を `.app` の `Contents/Library/LaunchAgents/` に同梱する。launchd の `KeepAlive` は付けない。単一インスタンスは loopback ポートの bind を使わず、UI 側は既に動作している UI のアクティブ化、daemon 側は local event store の writer lock で担う。自動再起動の判定材料は daemon プロセスの終了の仕方と起動期限の超過だけとし、`StartupFailureKind` による分岐を作らない。daemon の親プロセス終了時の終了経路は一括停止（`ShutdownCoordinator`）を経由しない。更新に伴う停止・再起動は既存の `ShutdownCoordinator` と統合する。updater は UI shell に残し Rust の切替制御と連携させる。起動監督・期限・再試行の判断、切替状態、停止完了の判定、起動対象・接続先の検証は Rust が所有する。CLI に daemon を単体起動する入口を作らない。

## 変えないもの

- 最小化状態で起動の設定が有効なときは、daemon の起動完了を待つ間のウィンドウも作らない。R-009 と B-010 を変更しない。人間が維持すると決めた条件である。
- Design 01 の「変えないもの」を維持する。一括停止（`ShutdownCoordinator`）の停止段階、期限、`RecoveryAction` の規則そのものと、daemon 側の単一インスタンスを local event store の writer lock で担う点を変えない。

## 未確定・リスク

- 自動判断（Requirements の Assumptions に記録、前周から維持）: R-015 が列挙する失敗の段階のうち「起動後の異常終了」だけ、段階と理由の表示を判定する受入条件が無かったため、B-020 へ段階と理由の表示を追加した。人間の確認は済んでいない。
- 自動判断（Requirements の Assumptions に記録、前周から維持）: R-025 が列挙する 3 つの失敗のうち「接続先またはリリースの不一致」だけ、終了できることを判定する受入条件が無かったため、B-023 と B-024 へ終了できることを追加した。人間の確認は済んでいない。
- 未決のまま残した要求は無い。`[DEFERRED]` で人間へ渡した件も無い。
- R-029 / B-033 は、macOS のログアウト・再起動・システム終了で R-006 と同じ停止を行い、かつそれらを妨げないことを求める。この停止は一括停止の完了を待つため、OS が終了要求に与える時間を超えると、R-006 の「完了してから終了する」と R-029 の「妨げない」の一方を満たせない組み合わせが生じうる。
