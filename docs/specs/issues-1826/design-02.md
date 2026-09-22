# Design 02

## 開始状態

差分の基準は base branch `main` の `b642f94d feat(workflow): プロセス在否に基づくNode再開と起動再試行を導入 (#1860)`、派生点も同 commit、作業 branch は `feat/issues/1826`。直前のDesignは `docs/specs/issues-1826/design-01.md` であり、その周の実装が未コミット差分として存在する。

Review後、`requirements.md` に R-012、`behavior.md` に B-018・B-019とR-012の対応が追加されている。解消または見送りとなったThreadはなく、13件の `[FIX_POLICY]` 付きThreadがopenである。

## 変える部分

- worktree削除の受理前副作用を除く: 所属不一致、locked、dirtyなどで削除を受理できない要求では実行木を変更せず、削除を受理した後だけ物理削除前にArchiveする。根拠: R-009、B-013、Thread `54b16849-ea2b-486d-a4e1-a18798f14650`。ルート: 委任
- linked worktree削除順のusecase境界検証を補う: 有効なlinked worktreeではArchiveをremoveより先に行い、Archive失敗時はworktreeとbranchの削除へ進まないことを検証可能にする。根拠: R-009、B-013、Thread `c06f92f6-082b-4922-bbe7-0dabc5437e06`。ルート: 委任
- Archive履歴の投影検証を補う: `manual`と`worktree_removed`の事実から履歴へ投影した`archived_at`と`archive_reason`が入力事実と一致することを検証可能にする。根拠: R-003、B-005、Thread `9d32b43b-b47c-49c4-9639-3e360bcd668e`。ルート: 委任
- Archive / Restore usecaseの境界検証を補う: 同一worktreeの複数実行木をすべてArchiveし、Restore前のworktree利用可能性確認が失敗した場合はArchiveを維持してRestore事実を記録しないことを検証可能にする。根拠: R-001〜R-004、R-011、B-001〜B-006、B-016、Thread `5c0cc9c6-680e-4f0a-85b6-6c99c44d804f`。ルート: 委任
- Archive snapshotのN+1読取を除く: 要求されたexecution tree ID群の最新Archive / Restore事実をboundedな一括queryで取得し、execution tree数に比例する独立SQLite queryを発行せず同じsnapshotを導出する。根拠: R-003、B-005、Thread `890aad51-169a-4c4a-a7cb-1bfa3060e69d`。ルート: 委任
- 並行Restoreを一意にする: 同じArchive期間へRestoreが並行して要求されても、対応するRestore事実を一度だけ記録する。根拠: R-003、R-011、B-016、Thread `fe64de8b-82c4-4c65-8750-44d42cb5dc0e`。ルート: 委任
- worktree削除中の状態変更を遮断する: Releashによる削除開始後は対象worktreeへの外部状態変更を拒否し、読み取りと、削除・GC内部のAbortからArchiveまでの遷移を継続できるようにする。根拠: R-012、B-011、B-013、B-018、B-019、Thread `d2b46389-20be-448f-b8aa-7797c73df881`。ルート: 委任
- Archive移行とGCのfull-recomputeを除く: 旧Archiveファイルが無い通常起動では全実行木をfoldせず、移行とGCは必要候補のboundedなsummary / read modelだけを読み、無関係な実行木の履歴読取失敗をdaemon起動全体へ波及させない。根拠: R-004、B-006、Thread `7c3bd91c-8f75-4e45-83fd-3c70a29f5c18`。ルート: 委任
- Archive事実の永続化形式をgateway境界へ戻す: domainのArchive型は理由と時刻の語彙だけを持ち、JSON名、既定値、後方互換decodeをgatewayで扱う。根拠: Thread `0feb54b7-0876-4975-b806-42538da1068c`。ルート: 委任
- Archiveの必須依存と配線を明示する: Archiveに必要なrepositoryとprocess停止能力を用途別の必須依存として扱い、既定のunavailable実装で欠落を隠さず、repositoryの生成・注入をcontrollerのcomposition rootへ戻す。根拠: Thread `253cc8e9-2169-4c1a-a36e-8c8b1ca3181c`。ルート: 委任
- 共通Archive契約の語彙をexecution treeへ揃える: Workflowと単独Sessionに共通するIDとArchive契約をexecution treeの語彙で表し、WorkflowExecution型はWorkflowDefinitionから開始された実行だけを表す。根拠: R-001、B-001、Thread `45174e31-3cda-4e4a-89e6-bfe5b4678b11`。ルート: 委任
- Archive遷移の二重表現を除く: Archiveの実遷移を正本のexecution treeモデルだけへ通し、保存されないAgentSession cloneの状態遷移と破棄されるlifecycle eventを生成しない。根拠: R-001、B-001、Thread `ce0cb9e0-5abb-4ee5-ae42-639180e5dbe0`。ルート: 委任
- 既存execution treeのGC消失判定をrepository単位にする: `repository_root`を持たない更新前の実行木も対象repositoryを判別し、対象repositoryを読める場合は無関係なrepositoryの読取失敗でArchiveを保留せず、対象自体を読めない場合だけ維持する。根拠: R-007、R-008、B-011、B-012、Thread `65f950f8-2822-42dc-bd39-4d9e1dcfed86`。ルート: 委任

## 固定するルート

固定する実装上の指定なし。

## 変えないもの

- Abortがworktreeフォルダの存在を要求せず、CommandのRetryが実行先フォルダの存在を要求する条件は維持する。理由: R-010、B-014、B-015はReviewで変更されていないため
- 起動時のWorkflow状態の常駐・復元方法と削除開始前の通常状態におけるworktree単位の起動排他（#1840）、実行木の自然完了の事実化と保存済み定義を読めない実行の扱い（#1836）、worktreeフォルダの削除方法と画面を待たせない変更（#1845）は変更しない。理由: triageでDesign 01の非対象範囲を維持すると決定済みのため

## 未確定・リスク

なし。
