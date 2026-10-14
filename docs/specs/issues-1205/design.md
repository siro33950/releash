# Design

## 変える部分
- 右カラムの面: 「タスク」の面を除き、先頭に Root の一覧の面を置いて、Root の一覧、ファイル、Source Control、Thread の順に並べる。根拠: R-001。ルート: 委任
- Root の一覧の行: 選んでいる worktree の実行木の Root のうち Archive されていないものを、種類のアイコン（状態の 3 値の色）、Session なら provider の表示名、タイトルで並べる。根拠: R-002。ルート: 「固定するルート」1
- Root の一覧に要る値の配信: Session の Root の provider を、サーバが Root ごとに配る。開始した Workflow の Root は、`StartWorkflow` が返す実行の ID を Root の `id` として配る今のサーバの規則（`server/src/domain/workspace_tree/visible.rs:169-173`）で特定し、サーバには足さない。根拠: R-002、R-004、R-019「Swift.app が別々に届いた値を突き合わせて求めない」。ルート: 委任
- 行からタブを開く: 行を押すと、Session なら Session タブ、Command なら Command タブ、Sequence と Fanout なら Workflow 管理タブを開く。Workflow 管理タブの中身は今の仮の表示のままにする。根拠: R-003。ルート: 委任
- New Session と New Workflow: 一覧の上端に置き、今の「+」の「Session」「Workflow」と同じ入力で開始する。成功したら、開始した Root について行を押したときと同じ振る舞いをする。根拠: R-004。ルート: 委任
- History: 一覧の上端に置き、Archive 済みの Session（実行木の `archivedSessions`）と provider の履歴（購読 `session-history`）を並べる。Archive 済みの Session は `RestoreAgentSession` と `DeleteAgentSession`、provider の履歴は `ResumeAgentSessionHistoryCandidate` を使う。Restore と Resume に成功したら、返った Node について行を押したときと同じ振る舞いをする。根拠: R-005。ルート: 委任
- 行の右クリックメニュー: 改名（`canRename`、`RenameWorkspaceSessionNode`）、Archive（`workflowCapabilities.canArchive`）、Resume（`canResumeSession`、`ResumeWorkspaceSessionNode`）を、値が真のときだけ出す。根拠: R-006、R-008。ルート: 「固定するルート」2
- Archive: すべての種類の Root で `ArchiveWorkspaceWorkflowExecution` を呼ぶ。`workflowCapabilities.canAbort` が真なら、実行を中止して Archive することの確認を先に出す。根拠: R-007。ルート: 「固定するルート」2
- 「+」の「開く」: Session、Workflow、Workflow 管理を外し、Terminal だけにする。根拠: R-009。ルート: 委任
- タブの保存項目: タブの種類に Command を足し、Session タブと Command タブに Node を指す ID を持たせて、サーバの UI 状態に保存する。根拠: R-010。ルート: 「固定するルート」3
- タブを閉じる: Session タブと Command タブを閉じても、Session と実行には何もせず、確認も出さない。根拠: R-011。ルート: 委任
- Session タブの TUI: Session の TUI を、#1759 の Terminal タブと同じ SwiftTerm の端末表示と購読の手順で表示し、入力を送る。タブを開いたら `OpenAgentSession` を呼ぶ。根拠: R-012。ルート: 「固定するルート」4
- Session を開く操作: `OpenAgentSession` は Archive 済みの Session を Restore せず、Archive されたままとする。Restore は History の `RestoreAgentSession` だけで行う。根拠: R-013、B-015。ルート: 委任
- Session を開けるかどうかの配信: domain の判定の結果を、購読 `agent-session` で配る。Swift.app はその値で、TUI、止まっている表示、判定できない表示、Archive されている表示を切り替える。止まっているときは `canResumeSession` が真なら Resume を出し、`ResumeWorkspaceSessionNode` を呼ぶ。根拠: R-013、R-014。ルート: 委任
- Command の shell と作業ディレクトリ: サーバが Command を実行するときに shell と作業ディレクトリを記録し、`node-detail` の Command の content で配る。根拠: R-015「shell と作業ディレクトリは、サーバが実行時に記録して配る」。ルート: 委任
- Command タブの表示: `node-detail` の値で、COMMAND 行（コマンド、作業ディレクトリ、shell、終了コード、所要時間、Retry、Abort）、STDOUT、STDERR、失敗理由を表示する。根拠: R-015、R-016。ルート: 委任
- Command タブの Retry と Abort: Retry は `canRetry` が真のとき出し、`RetryWorkspaceNode` を呼ぶ。Abort は、その Node の実行が中止できるかと中止する対象をサーバが `node-detail` で配り、中止できるときに出して `AbortWorkflow` を呼ぶ。根拠: R-017、R-019。ルート: 委任
- 過去の試行: 実行木の Node の `pastAttempts` を時刻（`updatedAt`）と失敗理由（`errorReason`）で並べ、押すとその試行の Node を指す Command タブを開く。サーバには足さない。根拠: R-018。ルート: 「固定するルート」5
- 回帰の E2E: `clients/macos/UITests/` に、Session タブに provider の TUI が表示されて入力できることと、Root が Command の実行で Command タブに出力と終了コードが表示されることを確かめるテストを足す。根拠: R-021、R-022。ルート: 委任

## 固定するルート
1. Root の一覧の行の見た目
   - サイドバーのカードの行（`docs/specs/issues-1204/requirements.md` の R-015）と同じく、種類のアイコンを状態の色で塗り、タイトルを並べる。Session の Root は、タイトルの前に provider の表示名を出す。
2. 行の右クリックメニュー
   - 置くのは改名、Archive、Resume だけにする。Restore と Delete は History からだけ行う。
   - 項目は、サーバが配る値が偽なら出さない（灰色にしない）。Apple の Human Interface Guidelines の Context menus に従う。
   - Archive は、すべての種類の Root で、実行木の単位の `ArchiveWorkspaceWorkflowExecution` を使う。`ArchiveAgentSession` は使わない。2 つの呼び出しは最後に同じ実行木の Archive を行うので、`docs/architecture/README.md:59` の「同じ操作の実装は 1 つに集約する」に従い、すべての Root に効く 1 本にする。可否は `workflowCapabilities.canArchive` で決める。
3. タブの保存項目
   - proto の `PaneTab` に Node を指す ID を、`PaneTabKind` に Command を足す。互換のある足し方にし、`buf.yaml` の breaking 検査（WIRE_JSON）を通す。
4. Session の TUI
   - terminal emulator は #1759 と同じく SwiftTerm を使う。
5. 過去の試行
   - 札に出すのは、今のデータにある時刻と失敗理由だけにする。attempt の番号と終了コードはサーバに足さない。

## 変えないもの
- React の画面: Session を開けるかどうかを lifecycle と terminalPresence から判定する今の処理（`clients/desktop/src/components/panels/AgentSessionPanel/AgentSessionPanel.tsx:76-82`）を含め、変えない。コードは変えないが、Archive 済みの Session のパネルを開いたときは、自動の Restore ではなく、今ある Archive の表示と Restore ボタンが出る（`clients/desktop/src/components/panels/AgentSessionPanel/AgentSessionPanel.tsx:129`、`:184-190`）。React は [16] #1765 で撤去する。
- `docs/specs/issues-1204/` と ISSUE 本文: 書き換えない。食い違う点は requirements.md の Context に書いたとおり、この spec を正とする。

## 未確定・リスク
- 過去の試行の Node の ID で `node-detail` を購読して、その試行の出力と終了コードが引けるかは、コードの経路（`server/src/adaptor/presenter/state_subscription_target.rs:70`、`server/src/adaptor/gateway/workspace_tree/query_service.rs:82-87`）から読んだだけで、実際には確かめていない。引けなければ、B-019 の「その試行の出力と終了コードが表示される」を満たすためにサーバの変更が要る。
