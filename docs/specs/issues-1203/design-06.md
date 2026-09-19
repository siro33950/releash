# Design 06

## 開始状態

- 差分の基準は `main` から派生した `feat/issues/1203` であり、派生点は `00b57d77`（`feat(daemon): Tauri非依存のバックエンドを抽出する (#1825)`）。開始状態は、派生点に Design 01 から Design 05 の実装（未コミットの差分）を加えた現行のワークツリーである。
- 直前の Design は `docs/specs/issues-1203/design-05.md`。Design 01 から Design 05 で実装済みの変更は本文書に再掲しない。
- Design 05 が変える部分に挙げた Thread `b3491499`（`src/App.tsx:121`）は、この周までに `resolved` で解消した。見送りとなった Thread は無い。この Issue の Thread は全 45 件で、うち 44 件が `resolved` である。
- `[FIX_POLICY]` を持つ open Thread は `f109c823`（`src-tauri/src/domain/daemon_supervision.rs:466`）の 1 件である。`[DEFERRED]`、`[REJECTED]` はいずれも 0 件である。
- Requirements と Behavior はこの周で変更していない。`R-001` から `R-035`、`B-001` から `B-043` に重複と欠番はなく、対応表は 35 行で全 Requirement ID を含み、表が参照する Behavior ID はすべて定義済みで、定義済みの Behavior はすべて表から参照されている。Open Question は残っていない。

## 変える部分

- 最小化起動中の回復可能な daemon 起動失敗でのウィンドウ非表示の維持: 最小化状態で起動の設定が有効なログイン時の起動で、初回の起動完了前に daemon が回復可能な失敗（異常終了または起動期限超過）で自動再試行へ入っても、ウィンドウを作成・表示しない。その後 daemon が Ready に達して復旧したときも通常ウィンドウを表示せず、メニューバーのアイコンだけの状態を保つ。R-015 / R-018 / B-021 が定める、自動再起動の対象外の失敗または試行上限到達時の理由と終了操作の表示は、この変更で失わない。根拠: Thread `f109c823`（最小化ログイン起動中に daemon が回復可能な失敗で自動再試行へ入るとウィンドウが開き、復旧後も通常ウィンドウが表示される）、R-009「最小化状態で起動の設定が有効なとき、ログイン時の起動ではウィンドウが表示されず、メニューバーのアイコンだけが出る」、R-031「最小化状態で起動の設定が有効なログイン時の起動では、このウィンドウも表示されない」、B-010、B-037。ルート: 委任

## 固定するルート

- 今周新たに人間が指定したルートは無い。`f109c823` の修正上のルート（起動完了前の回復可能な失敗でウィンドウを作らないための判定の所在、Phase と hidden の組み合わせの表現、復旧時に `has_failure_window` を `show_window` の根拠から外す方法、startup-failure ウィンドウの生成・破棄の扱い）はいずれも委任である。
- Design 03 で固定し Design 04 と Design 05 でも維持したルートを今周も維持する。ログイン項目の操作は `SMAppService` の `register()` / `unregister()` / `status()` だけを使い、`launchctl` の `enable` / `disable` / `print-disabled` を呼ぶ経路を作らない。承認を必要とする状態（requiresApproval）は「無効」として扱い、承認が必要な旨と System Settings への導線を表示し、起動経路はこの状態で失敗させない。後方互換のための機能を追加せず、既存の後方互換経路も残さない（Design 02 で固定）。
- Design 01 で固定し Design 02 から Design 05 でも維持した次のルートは今周も維持する。LaunchAgent は `SMAppService` の agent 方式を使い plist を `.app` の `Contents/Library/LaunchAgents/` に同梱する。launchd の `KeepAlive` は付けない。単一インスタンスは loopback ポートの bind を使わず、UI 側は既に動作している UI のアクティブ化、daemon 側は local event store の writer lock で担う。自動再起動の判定材料は daemon プロセスの終了の仕方と起動期限の超過だけとし、`StartupFailureKind` による分岐を作らない。daemon の親プロセス終了時の終了経路は一括停止（`ShutdownCoordinator`）を経由しない。更新に伴う停止・再起動は既存の `ShutdownCoordinator` と統合する。updater は UI shell に残し Rust の切替制御と連携させる。起動監督・期限・再試行の判断、切替状態、停止完了の判定、起動対象・接続先の検証は Rust が所有する。CLI に daemon を単体起動する入口を作らない。

## 変えないもの

- 最小化状態で起動の設定が有効なときは、daemon の起動完了を待つ間のウィンドウも作らない。R-009 と B-010 を変更しない。人間が維持すると決めた条件である。
- Design 01 の「変えないもの」を維持する。一括停止（`ShutdownCoordinator`）の停止段階、期限、`RecoveryAction` の規則そのものと、daemon 側の単一インスタンスを local event store の writer lock で担う点を変えない。

## 未確定・リスク

- 自動判断（Requirements の Assumptions に記録、前周から維持）: R-015 が列挙する失敗の段階のうち「起動後の異常終了」だけ、段階と理由の表示を判定する受入条件が無かったため、B-020 へ段階と理由の表示を追加した。人間の確認は済んでいない。
- 自動判断（Requirements の Assumptions に記録、前周から維持）: R-025 が列挙する 3 つの失敗のうち「接続先またはリリースの不一致」だけ、終了できることを判定する受入条件が無かったため、B-023 と B-024 へ終了できることを追加した。人間の確認は済んでいない。
- 未決のまま残した要求は無い。`[DEFERRED]` で人間へ渡した件も無い。
