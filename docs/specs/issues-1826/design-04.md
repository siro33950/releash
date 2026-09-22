# Design 04

## 開始状態

差分の基準は base branch `main` の `b642f94d feat(workflow): プロセス在否に基づくNode再開と起動再試行を導入 (#1860)`、派生点も同 commit、作業 branch は `feat/issues/1826`。直前のDesignは `docs/specs/issues-1826/design-03.md` であり、その周の実装が未コミット差分として存在する。

Design 03のReview後、Requirements・Behaviorの判断変更はない。Design 03に列挙したThreadのうち `65f950f8-2822-42dc-bd39-4d9e1dcfed86` 以外は解消済みであり、新規の `9088500b-9027-4156-b705-5b0d4b5d39dd` と合わせて2件の `[FIX_POLICY]` 付きThreadがopenである。入力でdismissedとされた5件は要求違反なしとして見送り、out-of-scopeとされた2件は今回差分が導入または悪化させていない既存問題として対象外である。

## 変える部分

- 起動時Recoveryとstartup GCの競合を解消する: 両者が同じ実行木を扱っても、GCによるAbort・Archiveの完了後にRecoveryがArchive前のactive状態を再登録せず、Commandを含むプロセスを起動しないようにする。根拠: R-002、R-007、R-008、B-002、B-011、Thread `9088500b-9027-4156-b705-5b0d4b5d39dd`。ルート: 委任
- repository情報を持たない既存実行木のGC消失判定を完成させる: 初回GCより前にGit登録とフォルダの両方が消えた任意位置のlinked worktreeでも対象repository単位で判定し、対象repositoryを読める場合は無関係なrepositoryの読取失敗でArchiveを保留せず、対象自体を読めない場合だけ維持する。根拠: R-007、R-008、B-011、B-012、Thread `65f950f8-2822-42dc-bd39-4d9e1dcfed86`。ルート: 委任

## 固定するルート

固定する実装上の指定なし。

## 変えないもの

- worktree削除対象がsymlinkへ置き換えられた場合の再帰削除先検証は変更しない。基準commitにも同じ経路があり、今回差分が導入または悪化させた問題ではないため、入力で本Issueの対象外とされている
- 正本の実行木aggregateと公開用read modelが同じ`ExecutionTree`名で表現されている既存構造は変更しない。基準commitにも同じ二重表現があり、今回差分は改名だけで問題を導入または悪化させていないため、入力で本Issueの対象外とされている

## 未確定・リスク

なし。
