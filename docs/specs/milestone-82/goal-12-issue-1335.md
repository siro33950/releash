# Goal: #1335 Resume を abort-only recovery から移行

まず `docs/specs/milestone-82/goal-common.md` を読み、そこに書かれた必読ドキュメント・設計判断・横断ルール・品質ゲートに従うこと。`gh issue view 1335 --repo siro33950/releash` で issue 本文を読むこと。

中断状態を再開可能な checkpoint として表現し、event log からの再構築で resume できるようにする。

## 実装内容

1. **中断状態の表現**: WorkflowExecution.status に再開可能な中断状態（interrupted。P2 の追加分）を導入する。crash / stale / explicit stop / orphan 検出で終端せずに interrupted になる。projection は event log から「最後に確定した NodeExecution」と次に実行すべき node を導出する。
2. **stop / resume typed command**（design.md §8.5）: **`StopExecution { execution_id }` を新設**し、explicit stop の入口を API（`POST .../stop`）/ CLI（`releash workflow stop <execution-id>`）/ UI アクションに揃える（Running / WaitingApproval → ExecutionInterrupted{reason: stop}。実行中 session の turn 中断・command process の kill を含む）。`ResumeExecution { execution_id }` は event log から状態を再構築して次の NodeExecution から再開する（未確定だった node は新しい反復回として再実行。session の再アタッチはせず、新 session を開始する）。許可状態集合: resume = Interrupted のみ、stop = Running / WaitingApproval、abort = Running / WaitingApproval / Interrupted。いずれも typed command として target validation（存在 / 状態 / worktree 整合）を行う。
3. **orphan recovery の変更**: 起動時の orphan 検出（orphan_recovery.rs）を「強制 abort」から「interrupted へ遷移させ、abort / resume を選べる状態にする」に変更する。既定で自動 abort しない。
4. **partial fanout failure（P5）**: fanout 途中で中断した場合、完了済み child の Artifact は event log から再利用し、未確定 child のみ再実行して fanout を完成させる。
5. **CLI / API / UI**: `releash workflow stop|resume <execution-id>`（local API 経由）、UI の interrupted 表示と stop / resume / abort アクション（同じ typed command boundary）を追加する。read model（#1331）に interrupted と再開点の情報を載せる。

## テスト

- crash / stale / explicit stop（StopExecution command）の各中断後に event log から再構築し、最後に確定した NodeExecution の次から resume できること（完了済み node が再実行されないこと）。
- stop / abort / resume の許可状態集合（終端状態への拒否、非 Interrupted への resume 拒否、stop 時の command process kill）。
- orphan recovery で強制 abort されず、abort / resume を typed command として選べること。
- partial fanout failure から resume し、完了済み child Artifact が再利用され未確定 child のみ再実行されること。
- resume の target validation（不存在 / 非 interrupted / 完了済みへの resume 拒否）。
