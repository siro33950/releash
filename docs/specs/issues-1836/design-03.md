# Design 03

## 開始状態

差分の基準は base branch `main` の `b642f94d feat(workflow): プロセス在否に基づくNode再開と起動再試行を導入 (#1860)` で、作業 branch `feat/issues/1836` の派生点も同じ commit である。直前のDesignは `docs/specs/issues-1836/design-02.md` であり、その実装は未コミット差分として存在する。

直前の周までにThread `ed57617f-29bd-423d-8629-9ceee2e7a5a1` と `535deb65-31ee-45c2-a9e6-16f2ef51dcaa` は解消済みである。今周の開始時点では、Design 02 の実装に対する `[FIX_POLICY]` 付きopen Thread `4b868aa7-ff46-4849-8464-333a6f6ca5eb`、`59fc794d-3e38-4aec-b3f6-7c7f2eb30057`、`90b1c19a-4d27-4fb9-a175-4da7a9659a87`、`4350362b-6edd-4e6e-93e5-7210b7245b83` が未解消である。見送りとなったThreadはない。

## 変える部分

- 起動時の重複した全fact履歴走査の解消: 定義非互換のAbort判定で、reconciliationとは別に同じ実行木の全fact履歴を無条件で再取得・再decodeする経路をなくす。根拠: R-003、B-003、Thread `4b868aa7-ff46-4849-8464-333a6f6ca5eb`。ルート: 委任
- 解釈不能な未完了実行の起動時Abort手順のレイヤー是正: tree列挙、起動時Abort usecase、後続reconciliationの順序をusecaseが所有し、controllerのcomposition rootで必要な依存を配線して、gatewayによる業務手順の所有をなくす。根拠: R-003、B-003、Thread `59fc794d-3e38-4aec-b3f6-7c7f2eb30057`。ルート: 委任
- 終端事実を持つ実行の単一経路での復元: 終端事実の有無で通常foldと簡易replayを切り替える二重実装をなくし、保存定義本文を解釈せず、Abort・composite・dynamic fanoutを含む公開状態を通常の事実解釈と同じ結果で復元する。根拠: R-002、B-002、Thread `90b1c19a-4d27-4fb9-a175-4da7a9659a87`。ルート: 委任
- 更新前のAbort済み隔離Nodeのworktree情報維持: 更新前に記録されたAbort済みの隔離Nodeでも、更新後の読み取りで従来と同じworktree branch/pathをNode詳細とArtifactに保持する。根拠: `docs/glossary/DOMAIN.md` の隔離worktree契約、Thread `4350362b-6edd-4e6e-93e5-7210b7245b83`。ルート: 委任

## 固定するルート

固定する実装上の指定なし

## 変えないもの

なし

## 未確定・リスク

なし
