# Design 05

## 開始状態

差分の基準は base branch `main` の `b642f94d feat(workflow): プロセス在否に基づくNode再開と起動再試行を導入 (#1860)`、派生点も同 commit、作業 branch は `feat/issues/1826`。直前のDesignは `docs/specs/issues-1826/design-04.md` であり、その周の実装が未コミット差分として存在する。

Design 04のReview後、Requirements・Behaviorの判断変更はない。Thread `9088500b-9027-4156-b705-5b0d4b5d39dd` は解消済みであり、継続する `65f950f8-2822-42dc-bd39-4d9e1dcfed86` と新規の `6696c58d-12be-4d3f-b3d3-49c29a68c5ab` の2件の `[FIX_POLICY]` 付きThreadがopenである。入力でdismissedとされた3件はRequirements・Behaviorの観測可能な違反なしとして見送り、out-of-scopeとされた1件は今回差分が導入または悪化させていない既存問題として対象外である。

## 変える部分

- 削除を受理できないworktreeの実行木を維持する: 先行するworktree mutationの終了待ち中にdirtyになる場合を含め、所属不一致、locked、dirtyなどによりworktreeまたはbranchの削除を受理できないときは実行木をAbort／Archiveせず、削除を受理できる場合だけ物理削除より先にArchiveする。根拠: R-009、B-013、Thread `6696c58d-12be-4d3f-b3d3-49c29a68c5ab`。ルート: 委任
- repository情報を持たない既存実行木のGC消失判定を完成させる: 初回GCより前にGit登録とフォルダの両方が消えた任意位置のlinked worktreeでも対象repository単位で判定し、対象repositoryを読める場合は無関係なrepositoryの読取失敗でArchiveを保留せず、対象自体を読めない場合だけ維持する。根拠: R-007、R-008、B-011、B-012、Thread `65f950f8-2822-42dc-bd39-4d9e1dcfed86`。ルート: 委任

## 固定するルート

固定する実装上の指定なし。

## 変えないもの

- 単独Sessionの実行木をWorkflow固有のcapability型とWorkflowExecution向けcommandで公開する構造は変更しない。共通の実行木Archiveへ到達し、R-001／B-001の観測可能な違反がないため
- テスト専用に残る旧AgentSession Archive／Restore遷移と旧writer変換は変更しない。productionの呼び出しがなく、Requirements・Behaviorの観測可能な違反がないため
- AgentSessionのArchive／Restore APIが現在の実装で使わない値を必須入力とする契約は変更しない。現在の操作結果にRequirements・Behaviorの観測可能な違反がないため
- 変更対象のdomain fact型に残るserde永続形式依存は変更しない。基準commitから存在し、今回差分が導入または悪化させていないため

## 未確定・リスク

なし。
