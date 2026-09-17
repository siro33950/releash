# Design 05

## 開始状態

- 差分の基準は `main` から派生した `feat/issues/1203` であり、派生点は `00b57d77`（`feat(daemon): Tauri非依存のバックエンドを抽出する (#1825)`）。開始状態は、派生点に Design 01 から Design 04 の実装（未コミットの差分）を加えた現行のワークツリーである。
- 直前の Design は `docs/specs/issues-1203/design-04.md`。Design 01 から Design 04 で実装済みの変更は本文書に再掲しない。
- Design 04 が変える部分に挙げた 2 件の Thread（`1910f237` / `031fe129`）は、この周までに `resolved` で解消した。見送りとなった Thread は無い。この Issue の Thread は全 44 件で、うち 43 件が `resolved` である。
- `[FIX_POLICY]` を持つ open Thread は `b3491499`（`src/App.tsx:121`）の 1 件である。`[DEFERRED]`、`[REJECTED]` はいずれも 0 件である。
- Requirements と Behavior はこの周で変更していない。`R-001` から `R-035`、`B-001` から `B-043` に重複と欠番はなく、対応表は 35 行で全 Requirement ID を含み、表が参照する Behavior ID はすべて定義済みで、定義済みの Behavior はすべて表から参照されている。Open Question は残っていない。

## 変える部分

- 復元に必要な状態の初回取得が失敗したときの回復: 復元に必要な状態の初回取得が一時的に失敗しても、UI が復元待ちへ無期限に留まらないようにする。現在の状態の再取得と反映が成立した後に受付を再開し、回復できない場合も待機表示のまま原因不明で操作不能にならないようにする。根拠: Thread `b3491499`（受入条件は、復元に必要な状態の初回取得が一時的に失敗しても UI は Restoring へ無期限に留まらず、現在の状態の再取得と反映が成立した後に受付が再開されること、回復できない場合も待機表示のまま原因不明で操作不能にならないこと）、R-004「daemon が異常終了した場合、UI は待機間隔と試行上限の範囲で daemon を再び起動し、再接続した後は現在の状態が画面へ反映される」・B-004、R-023「新 daemon が Ready に達し、接続先とリリースの一致と認証済みのクライアント ws 接続を確認し、必要な状態を再取得した後に受付を再開する」、R-026 / B-030。ルート: 委任

## 固定するルート

- 今周新たに人間が指定したルートは無い。`b3491499` の修正上のルート（取得失敗の検知箇所、再取得の契機、復元完了の判定の所在、回復できない場合の表示手段）はいずれも委任である。
- Design 03 で固定し Design 04 でも維持したルートを今周も維持する。ログイン項目の操作は `SMAppService` の `register()` / `unregister()` / `status()` だけを使い、`launchctl` の `enable` / `disable` / `print-disabled` を呼ぶ経路を作らない。承認を必要とする状態（requiresApproval）は「無効」として扱い、承認が必要な旨と System Settings への導線を表示し、起動経路はこの状態で失敗させない。後方互換のための機能を追加せず、既存の後方互換経路も残さない（Design 02 で固定）。
- Design 01 で固定し Design 02 から Design 04 でも維持した次のルートは今周も維持する。LaunchAgent は `SMAppService` の agent 方式を使い plist を `.app` の `Contents/Library/LaunchAgents/` に同梱する。launchd の `KeepAlive` は付けない。単一インスタンスは loopback ポートの bind を使わず、UI 側は既に動作している UI のアクティブ化、daemon 側は local event store の writer lock で担う。自動再起動の判定材料は daemon プロセスの終了の仕方と起動期限の超過だけとし、`StartupFailureKind` による分岐を作らない。daemon の親プロセス終了時の終了経路は一括停止（`ShutdownCoordinator`）を経由しない。更新に伴う停止・再起動は既存の `ShutdownCoordinator` と統合する。updater は UI shell に残し Rust の切替制御と連携させる。起動監督・期限・再試行の判断、切替状態、停止完了の判定、起動対象・接続先の検証は Rust が所有する。CLI に daemon を単体起動する入口を作らない。

## 変えないもの

- 最小化状態で起動の設定が有効なときは、daemon の起動完了を待つ間のウィンドウも作らない。R-009 と B-010 を変更しない。人間が維持すると決めた条件である。
- Design 01 の「変えないもの」を維持する。一括停止（`ShutdownCoordinator`）の停止段階、期限、`RecoveryAction` の規則そのものと、daemon 側の単一インスタンスを local event store の writer lock で担う点を変えない。

## 未確定・リスク

- 自動判断（Requirements の Assumptions に記録、前周から維持）: R-015 が列挙する失敗の段階のうち「起動後の異常終了」だけ、段階と理由の表示を判定する受入条件が無かったため、B-020 へ段階と理由の表示を追加した。人間の確認は済んでいない。
- 自動判断（Requirements の Assumptions に記録、前周から維持）: R-025 が列挙する 3 つの失敗のうち「接続先またはリリースの不一致」だけ、終了できることを判定する受入条件が無かったため、B-023 と B-024 へ終了できることを追加した。人間の確認は済んでいない。
- 未決のまま残した要求は無い。`[DEFERRED]` で人間へ渡した件も無い。
