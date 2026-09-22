# Design 06

## 開始状態

差分の基準は base branch `main` の `b642f94d feat(workflow): プロセス在否に基づくNode再開と起動再試行を導入 (#1860)`、派生点も同 commit、作業 branch は `feat/issues/1826`。直前のDesignは `docs/specs/issues-1826/design-05.md` であり、その周の実装が未コミット差分として存在する。

Design 05のReview後、Requirements・Behaviorの判断変更はない。Thread `6696c58d-12be-4d3f-b3d3-49c29a68c5ab` は解消済みであり、継続する `65f950f8-2822-42dc-bd39-4d9e1dcfed86` と新規の `a6694c8e-7503-4b41-b911-c8f5f0a1c2c5`、`ed3f4035-e0a3-4768-9c07-d4dd87359933` の3件の `[FIX_POLICY]` 付きThreadがopenである。入力でdismissedとされた3件はRequirements・Behaviorの観測可能な違反なしとして見送られている。

## 変える部分

- Archiveと自然完了の競合を解消する: Archiveがactive状態を確認した後に同じ実行木が自然完了しても、終了状態を変えずにArchive済みとし、一時的な競合だけを理由に未Archiveのまま残さない。根拠: R-002、B-003、Thread `a6694c8e-7503-4b41-b911-c8f5f0a1c2c5`。ルート: 委任
- 外部入力pathによる永続worktree operationファイルの無制限増加を防ぐ: 未認可または不正な一意pathの状態変更要求を反復しても永続ファイル数が無制限に増えず、正規のworktreeに対する削除開始後の状態変更拒否を維持する。根拠: R-012、B-018、Thread `ed3f4035-e0a3-4768-9c07-d4dd87359933`。ルート: 委任
- repository情報を持たない既存実行木のGC消失判定を完成させる: 初回GCより前にGit登録とフォルダの両方が消えた任意位置のlinked worktreeでも対象repository単位で判定し、対象repositoryを読める場合は無関係なrepositoryの読取失敗でArchiveを保留せず、対象自体を読めない場合だけ維持する。根拠: R-007、R-008、B-011、B-012、Thread `65f950f8-2822-42dc-bd39-4d9e1dcfed86`。ルート: 委任

## 固定するルート

固定する実装上の指定なし。

## 変えないもの

なし。

## 未確定・リスク

なし。
