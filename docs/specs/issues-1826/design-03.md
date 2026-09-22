# Design 03

## 開始状態

差分の基準は base branch `main` の `b642f94d feat(workflow): プロセス在否に基づくNode再開と起動再試行を導入 (#1860)`、派生点も同 commit、作業 branch は `feat/issues/1826`。直前のDesignは `docs/specs/issues-1826/design-02.md` であり、その周の実装が未コミット差分として存在する。

Design 02のReview後、Requirements・Behaviorの判断変更はない。Thread `c06f92f6-082b-4922-bbe7-0dabc5437e06`、`9d32b43b-b47c-49c4-9639-3e360bcd668e`、`890aad51-169a-4c4a-a7cb-1bfa3060e69d`、`fe64de8b-82c4-4c65-8750-44d42cb5dc0e`、`54b16849-ea2b-486d-a4e1-a18798f14650`、`7c3bd91c-8f75-4e45-83fd-3c70a29f5c18` は解消済みであり、15件の `[FIX_POLICY]` 付きThreadがopenである。

## 変える部分

- 旧Session Archive移行のpage境界検証を補う: 旧Session Archiveが128件のpage境界を越えても全件移行され、129件目以降も時刻・理由・終了状態を維持して移行候補から外れることをusecase境界で検証可能にする。根拠: R-004、B-006、Thread `f58a1668-f984-41f4-9505-fc987a322498`。ルート: 委任
- startup GCのproduction結線検証を補う: productionのstartup GC passへ非空の消失worktree候補を渡したとき、対応する実行木がArchiveされ、終了していない対象がAbortされることを結線境界で検証可能にする。根拠: R-007、R-008、B-011、Thread `af53f7d3-91fd-405c-b0c9-fec4b445ef4f`。ルート: 委任
- 旧Archive移行のdaemon起動結線検証を補う: 旧Archiveファイルを含むdaemon起動経路で、事実移行、未終了対象のAbort、旧ファイル削除が実行されることを検証可能にする。根拠: R-004、B-006、Thread `1bb4b062-cd6e-448d-825e-49e53051d529`。ルート: 委任
- GC Archive手順のusecase境界検証を補う: GCの候補page反復、repository単位の維持判定、消失候補のArchiveを同階層のusecase境界で検証可能にする。根拠: R-007、R-008、B-010〜B-012、Thread `2abfbf7a-3c28-4116-a463-0404fff317e6`。ルート: 委任
- worktree単位Archiveの候補処理をboundedにする: 対象worktreeの候補だけをboundedに処理し、対象外の壊れたpathや全実行木件数の増加によって対象worktreeのArchiveを失敗させず、全候補を保持しない。根拠: R-009、B-013、Thread `0adb93af-bfc6-43b8-825e-b70b3b04aa77`。ルート: 委任
- 単独SessionのArchive / Restoreエラー分類を維持する: 共通Archive / Restoreから返る競合、不正操作、破損、保存不能を既存のusecaseエラー分類へ対応付け、すべてを`StorageUnavailable`として外部化しない。根拠: R-012、B-018、Thread `18dd507b-f20e-4a48-a86e-7e5e47dc59a3`。ルート: 委任
- GCを候補単位の失敗から継続可能にする: 一つの消失worktree候補のArchiveが失敗しても失敗を記録して独立した後続候補の走査を続け、後続の消失worktreeの実行木をArchiveできるようにする。根拠: R-007、R-008、B-011、Thread `00939e58-46ab-4d42-9857-6e91f9e7daa9`。ルート: 委任
- 単独Session Archiveの確認フォールバックを除く: backend契約から確認フォールバックを取り除き、すべてのArchive入口を確認用追加入力なしの共通実行木Archiveへ通して事実ログに記録する。根拠: R-001、R-006、B-001、B-009、Thread `4ef05d20-e6d8-4e33-9f75-9a1c5a412a85`。ルート: 委任
- Archive / Restore usecaseの境界検証を完成させる: 同一worktreeの複数実行木をすべてArchiveし、Restore前のworktree利用可能性確認が失敗した場合はArchiveを維持してRestore事実を記録しないことをusecase境界で検証可能にする。根拠: R-001〜R-004、R-011、B-001〜B-006、B-016、Thread `5c0cc9c6-680e-4f0a-85b6-6c99c44d804f`。ルート: 委任
- 削除中worktreeの外部状態変更拒否を全入口で共有する: 別processのReview CLIを含む外部状態変更要求を、削除中worktreeに対する同じ受理条件へ通し、読み取りと削除・GC内部のAbortからArchiveまでの遷移は継続できるようにする。根拠: R-012、B-011、B-013、B-018、B-019、Thread `d2b46389-20be-448f-b8aa-7797c73df881`。ルート: 委任
- 旧Archive事実の後方互換判断をgateway境界へ移す: `archivedAt`が欠けた旧事実の時刻補完をgatewayのdecode境界で完了し、domainのArchive事実と状態導出へ欠落値の既定判断を残さない。根拠: Thread `0feb54b7-0876-4975-b806-42538da1068c`。ルート: 委任
- AgentSession利用者ごとの必須依存を分離する: Session起動の利用者へ不要なmutation admission、実行木lock、Archive、Restore、cache解放を要求せず、各操作に必要な能力だけを必須依存として明示する。根拠: Thread `253cc8e9-2169-4c1a-a36e-8c8b1ca3181c`。ルート: 委任
- WorkflowExecution単一取得の境界を揃える: 単一取得でもWorkflowとして起動した実行木だけを`WorkflowExecutionSummary`へ投影し、単独SessionをWorkflowExecutionとして返さない。根拠: R-001、B-001、Thread `45174e31-3cda-4e4a-89e6-bfe5b4678b11`。ルート: 委任
- Archive / Restore遷移をexecution tree集約へ通す: Archive / Restoreの受理判定と状態遷移を正本のexecution treeモデルへ適用し、gatewayが独自に受理判定して事実を追記する二重表現を除く。根拠: R-001、B-001、Thread `ce0cb9e0-5abb-4ee5-ae42-639180e5dbe0`。ルート: 委任
- repository情報を持たない既存実行木のGC判定を対象repository単位にする: 任意位置のlinked worktreeを含む更新前の実行木でも対象repositoryを判別し、対象repositoryを読める場合は無関係なrepositoryの読取失敗でArchiveを保留せず、対象自体を読めない場合だけ維持する。根拠: R-007、R-008、B-011、B-012、Thread `65f950f8-2822-42dc-bd39-4d9e1dcfed86`。ルート: 委任

## 固定するルート

固定する実装上の指定なし。

## 変えないもの

- worktree削除対象がsymlinkへ置き換えられた場合の再帰削除先検証は変更しない。基準commitにも同じ経路があり、今回差分が導入または悪化させた問題ではないため、入力で本Issueの対象外とされている

## 未確定・リスク

なし。
