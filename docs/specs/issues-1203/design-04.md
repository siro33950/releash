# Design 04

## 開始状態

- 差分の基準は `main` から派生した `feat/issues/1203` であり、派生点は `00b57d77`（`feat(daemon): Tauri非依存のバックエンドを抽出する (#1825)`）。開始状態は、派生点に Design 01 から Design 03 の実装（未コミットの差分）を加えた現行のワークツリーである。
- 直前の Design は `docs/specs/issues-1203/design-03.md`。Design 01 から Design 03 で実装済みの変更は本文書に再掲しない。
- Design 03 が列挙した 22 件の Thread のうち 20 件（`e047d2e8` / `6b1ef363` / `9b7b197a` / `1760c30d` / `3b33b0cc` / `c72c3681` / `d5b1e9cc` / `e9d6564d` / `fb616b0c` / `7d2cf803` / `86e9d4de` / `297f1d6f` / `7381bf20` / `c56535b7` / `47f8f92e` / `3da32a69` / `735485ae` / `ab09a220` / `8fefa1c7` / `95cfd747`）は、この周までに `resolved` で解消した。見送りとなった Thread は無い。`[DEFERRED]`、`[REJECTED]` はいずれも 0 件である。
- `[FIX_POLICY]` を持つ open Thread は 2 件（`1910f237` / `031fe129`）であり、いずれも Design 03 でも変える部分に挙げた件が `[STILL_OPEN]` で残ったものである。`1910f237` は最新の `[FIX_POLICY]` で対象が起動表示規則の所有へ絞られている。
- Requirements と Behavior はこの周で変更していない。`R-001` から `R-035`、`B-001` から `B-043` に重複と欠番はなく、対応表は 35 行で全 Requirement ID を含み、表が参照する Behavior ID はすべて定義済みで、定義済みの Behavior はすべて表から参照されている。Open Question は残っていない。

## 変える部分

- 起動中と初回 Ready のウィンドウ表示可否の規則の所有: ウィンドウを表示するかどうかの規則を domain の判断として一箇所に置き、controller が最小化状態で起動の指定と初回 Ready を独自に評価して表示を決める形を無くす。根拠: Thread `1910f237`（受入条件は、起動中と初回 Ready のウィンドウ表示可否の規則が domain の判断として一箇所にあり、controller が hidden と初回 Ready を独自に評価して表示を決めないこと。R-009 / R-031 と B-010 / B-036 / B-037 の観測可能な結果は変えない）。ルート: 委任
- ログイン項目の登録希望と実状態の規則の所在: 実状態と保存する登録希望の区別、およびどちらを永続化するかの規則を Rust に置き、frontend の代入がその正本にならないようにする。同じ backend 操作を別の client surface から使っても同じ結果になるようにする。根拠: Thread `031fe129`、R-030「ログイン時に起動の登録が macOS の承認を必要とする状態のとき、ログイン時に起動の設定は無効として示され、承認が必要であることと承認を行う場所への導線が示される」、B-034 / B-035（観測可能な結果は変えない）。ルート: 委任

## 固定するルート

- 今周新たに人間が指定したルートは無い。
- Design 03 で維持したルートを今周も維持する。ログイン項目の操作は `SMAppService` の `register()` / `unregister()` / `status()` だけを使い、`launchctl` の `enable` / `disable` / `print-disabled` を呼ぶ経路を作らない。承認を必要とする状態（requiresApproval）は「無効」として扱い、承認が必要な旨と System Settings への導線を表示し、起動経路はこの状態で失敗させない。後方互換のための機能を追加せず、既存の後方互換経路も残さない（Design 02 で固定）。
- Design 01 で固定し Design 02・Design 03 でも維持した次のルートは今周も維持する。LaunchAgent は `SMAppService` の agent 方式を使い plist を `.app` の `Contents/Library/LaunchAgents/` に同梱する。launchd の `KeepAlive` は付けない。単一インスタンスは loopback ポートの bind を使わず、UI 側は既に動作している UI のアクティブ化、daemon 側は local event store の writer lock で担う。自動再起動の判定材料は daemon プロセスの終了の仕方と起動期限の超過だけとし、`StartupFailureKind` による分岐を作らない。daemon の親プロセス終了時の終了経路は一括停止（`ShutdownCoordinator`）を経由しない。更新に伴う停止・再起動は既存の `ShutdownCoordinator` と統合する。updater は UI shell に残し Rust の切替制御と連携させる。起動監督・期限・再試行の判断、切替状態、停止完了の判定、起動対象・接続先の検証は Rust が所有する。CLI に daemon を単体起動する入口を作らない。

## 変えないもの

- 最小化状態で起動の設定が有効なときは、daemon の起動完了を待つ間のウィンドウも作らない。R-009 と B-010 を変更しない。人間が維持すると決めた条件である。
- Design 01 の「変えないもの」を維持する。一括停止（`ShutdownCoordinator`）の停止段階、期限、`RecoveryAction` の規則そのものと、daemon 側の単一インスタンスを local event store の writer lock で担う点を変えない。

## 未確定・リスク

- 自動判断（Requirements の Assumptions に記録、前周から維持）: R-015 が列挙する失敗の段階のうち「起動後の異常終了」だけ、段階と理由の表示を判定する受入条件が無かったため、B-020 へ段階と理由の表示を追加した。人間の確認は済んでいない。
- 自動判断（Requirements の Assumptions に記録、前周から維持）: R-025 が列挙する 3 つの失敗のうち「接続先またはリリースの不一致」だけ、終了できることを判定する受入条件が無かったため、B-023 と B-024 へ終了できることを追加した。人間の確認は済んでいない。
- 未決のまま残した要求は無い。`[DEFERRED]` で人間へ渡した件も無い。
