# Design 07

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `feat/issues/1861` の `81ec380b`。Design 01〜06 の周の実装は `8eff7b47`（`fix(workspace): 更新中の一覧保持と全体再取得を実装 (#1861)`）としてコミット済みで、作業ツリーの未コミット変更は `docs/specs/issues-1861/requirements.md` と `docs/specs/issues-1861/behavior.md` の2件だけである。Spec 工程ではコードを変更していないため、`8eff7b47` の実装をこの周の開始状態とする。
- 直前の Design は `docs/specs/issues-1861/design-06.md`（入力の `context.design_path` と一致）。
- open Thread は6件（35a998c2-07b9-44c4-ade5-37fc5069d579、ea71ac18-5360-4527-bcef-0b8abe6bf34f、7d95e1ef-e703-43f6-81a0-d4e808e175fb、bd89e8ce-d93f-4a20-84a2-88585aa0ef61、16600fee-8ad7-471c-a828-e795190e14d9、1cdbeeec-41dd-4651-a790-b995b13d22e7）で、いずれも `[FIX_POLICY]` が付いている。`[REJECTED]`・`[DEFERRED]` にした Thread はない。前の周までに扱った Thread は開始時点で open ではない。
- Requirements・Behavior はこの周の前段（triage_requirements_and_behavior）が更新済みで、Design では変更していない。R-005 の更新対象へ「各 Repository の PR 情報」を加え、R-017〜R-020 と B-024〜B-027 を追加し、B-006・B-021 の WHEN・THEN を改訂している。R-001〜R-020 と B-001〜B-027 に欠番・重複はなく、対応表は全 Requirement ID を記載して各行に Behavior ID が対応している。Assumptions / Open Questions は「なし」である。
- Design 06 の「変える部分」2件は開始状態で満たされている。Repository を対象とする RPC の失敗は `src/hooks/useWorkspaceList.ts:101-105` で `repositoryErrors` に入り、Workspaces 一覧全体の `error` には入らない。全体の失敗表示の解消は同:86-92 のとおり `worktreePath` と `repoPath` のいずれも未指定の成功に限られる。
- Design 06 の「未確定・リスク」は「なし」であり、引き継ぐ事項はない。

## 変える部分

- 一覧の取得と PR 情報の取得の分離: Workspaces 一覧の更新経路から PR 情報の取得を外し、一覧の表示が PR 情報の取得を待たないようにする。開始状態では `src-tauri/src/usecase/workspace_tree/list.rs:107-128` の `refresh_branches` が branches の取得後に同期の `self.query.pr_status(path)`（同:109）を呼び、その完了まで同:129 の `complete_branches` へ進まない。根拠: Thread 35a998c2-07b9-44c4-ade5-37fc5069d579（blocking、`[FIX_POLICY]`）、R-017「一覧の表示は PR 情報の取得を待たない。PR 情報が未取得の対象でも、取得できた一覧をその時点で表示し、PR 情報は取得できた時点で一覧の表示へ反映する」、R-018、B-024、B-025。ルート: 分離は Rust 側で行い、派生点 `81ec380b` のように frontend で PR 情報を重ね直す形へは戻さない。PR 情報の state の持ち方、取得の契機、snapshot の形、Tauri command と local API の経路の形は委任
- PR 情報の取得の Repository 間の独立: ある Repository の PR 情報の取得が、他の Repository の PR 情報の一覧への反映を待たせないようにする。開始状態では `src-tauri/src/usecase/workspace_tree/list.rs:90-95` の `join_all` が全 Repository の `refresh_branches` を一つの task で poll し、同:109 の `pr_status` が同期であるため Repository 数だけ直列化する。根拠: Thread ea71ac18-5360-4527-bcef-0b8abe6bf34f（blocking、`[FIX_POLICY]`）、R-019「ある Repository の PR 情報の取得は、他の Repository の PR 情報の一覧への反映を待たせない」、B-026。ルート: 分離は Rust 側で行い、frontend で PR 情報を重ね直す形へは戻さない。それ以外の実装方法は委任
- 未開始の全体更新の要求の統合: 更新の処理中に同じ全体更新の契機が複数回生じても、まだ開始していない全体更新の要求を一つに統合する。開始状態では `src/hooks/useWorkspaceList.ts:50-51` の `request` が呼出しごとに `requestsRef.current.then(...)` を作って同:111 で差し替えるだけで、未開始の要求を統合する状態を持たない。根拠: Thread 7d95e1ef-e703-43f6-81a0-d4e808e175fb（blocking、`[FIX_POLICY]`）、R-020「更新の処理中に同じ全体更新の契機が複数回生じても、まだ開始していない全体更新の要求は一つに統合される」、B-027。ルート: 委任
- Worktree 作成モーダルの入力状態の保持: 登録 Repository の内容が変わらない一覧の更新で、`CreateWorktreeModal` の選択・filter・取得済み候補が初期化されないようにする。開始状態では `src/components/workspace/WorkspaceList.tsx:1733-1734` が render ごとに `repositories` の fallback 配列と `listedRepoPaths` を新規生成して同:1824-1826 で開いているモーダルへ渡し、`src/components/workspace/CreateWorktreeModal.tsx:94-102` の effect が `[open, repoPaths]` の参照変更ごとに mode・selectedBranches・baseBranch・filter・error・selectedRepoPath を初期化する。同:104-136 の branch 取得 effect も `selectedRepoPath` の変化で再実行する。根拠: Thread bd89e8ce-d93f-4a20-84a2-88585aa0ef61（blocking、`[FIX_POLICY]`）、`requirements.md` Scope / 変更しない対象「Repository の追加・削除、Worktree の作成・削除の操作」。ルート: 委任
- 明示再走査の終端: 走査中に generation が更新され続けても `rescan_branches` が終端し、`scan_lock` を保持したまま worker と後続の再走査を止め続けないようにする。開始状態では `src-tauri/src/usecase/repository_state/service.rs:118-134` が `scan_lock` を保持したまま上限のない `loop` を回り、`src-tauri/src/usecase/repository_state/worktree.rs:192` の `invalidate` は `scan_lock` を取らずに generation を進められる。根拠: Thread 16600fee-8ad7-471c-a828-e795190e14d9（blocking、`[FIX_POLICY]`）、R-012「更新が成功・失敗のいずれで終わった後も、再び操作できる」、B-016。ルート: 委任
- 購読に紐づかない監視 state を残さない: 明示再走査だけが実行され watch の購読が成立しない経路でも、未購読の `WorktreeState`・filesystem watcher・worker が蓄積しないようにする。開始状態では `src-tauri/src/usecase/repository_state/service.rs:117` が `rescan_branches` から `ensure_watching(repo_path)` を購読情報なしで呼び、同:265-307 が新しい state を生成して map へ登録する一方、同:217-245 の `stop_watching` は subscription を release できた state だけを削除する。根拠: Thread 1cdbeeec-41dd-4651-a790-b995b13d22e7（blocking、`[FIX_POLICY]`）、`AGENTS.md`「状態の所有者を明確にする」「full-retention 設計を避ける」。ルート: 委任
- `include_deleting_worktrees` と `classify_branch_cards` の合成の一本化: 二つを続けて呼ぶ組み立てを一箇所へ寄せる。開始状態では `src-tauri/src/usecase/repository_state/service.rs:131-132`、同:192-193、同:205-206 の3箇所で同じ順序の組み立てを繰り返している。根拠: human.decisions D-05。ルート: 共通化先は委任
- `classify_branch_cards` の domain への移設: branch card の分類規則を domain が持つようにする。開始状態では `src-tauri/src/usecase/repository_query_service.rs:41-58` にあり、`WorktreeInventoryEntry::matches_isolated_identity_rule` による分類を usecase 層が所有している。根拠: human.decisions D-10。ルート: domain 内の配置先は委任
- 呼び出し元が消えた branch 一覧 command の削除: `list_branches_with_status` と `list_branches_with_status_snapshot` を削除する。開始状態では `src-tauri/src/adaptor/controller/client/repository/shared.rs:506`・同:536 が両方を command として登録し、`src-tauri/src/adaptor/controller/client/repository/worktree.rs:31-44` が実体へ委譲している。根拠: human.decisions D-06。ルート: 委任
- R-010 の3状態の分類の Rust 側での所有: 初回取得中、初回取得の失敗、正常に取得した空の結果の区別を Rust 側が持つ。開始状態では `src-tauri/src/usecase/workspace_tree/list.rs:15-18` の `WorkspaceListStatusDto` が `loaded: bool` と `error: Option<String>` だけを渡し、`src/components/workspace/WorkspaceList.tsx:1669-1684` が `!status.loaded && !error`、`status.loaded && !error && branches.length === 0`、`error` の組み合わせで3状態を組み立てている。根拠: human.decisions D-07。ルート: 3状態を表す型と frontend への渡し方は委任
- 更新失敗の state の所有者の一本化: 更新失敗を持つ場所を一つにする。開始状態では Rust 側 snapshot の `status.error` と、frontend の `error`・`repositoryErrors`・`worktreeErrors`（`src/hooks/useWorkspaceList.ts:40-46`）が並存し、`src/components/workspace/WorkspaceList.tsx:1617` の `rpcError ?? status.error` と同:1736 の `model.error ?? model.snapshot?.status.error` で合成している。根拠: human.decisions D-08。ルート: 一本化の方法と RPC 自体が失敗した場合の扱いは委任
- `WorkspaceListRefresh` の失敗表現: 失敗を保持する表現を改める。開始状態では `src-tauri/src/domain/workspace_tree/refresh.rs:7`・同:21-31・同:41 が `error: Option<String>` と `Result<T, String>` で失敗を持つ。根拠: human.decisions D-09。ルート: 失敗表現の型は委任

## 固定するルート

- PR 情報の分離の実施場所: 一覧の取得と PR 情報の取得の分離は Rust 側で行う。派生点 `81ec380b` のように frontend で PR 情報を重ね直す形へは戻さない。範囲: 変える部分の「一覧の取得と PR 情報の取得の分離」と「PR 情報の取得の Repository 間の独立」。粒度: 人間が示したこの2点のみで、実装方法は委任（human.decisions D-01）。
- Design 01 で固定した次の4つを維持する（human.constraints）。①全体の更新手順と結果の判定を Rust 側に置き、既存の `RepositoryStateService` の走査処理と Workspace の一覧取得処理を再利用する。Frontend は受け取った一覧・更新状態・エラーを表示するだけにする。②一覧を保持する変更と、Session・Workflow の再取得経路の変更を併せて行う。③更新対象は Workspaces の一覧情報に限り、表示中の Session や実行中の Workflow の継続を保つ。④手動更新と自動更新で内部の経路を分けない。
- この周で新しく固定する実装上の指定はない。

## 変えないもの

- Design 01 の「変えないもの」3件を維持する。自動更新の周期と起動契機、手動更新に固有の時間上限を設けず応答が返らない場合の打ち切りを既存 client の deadline に委ねること、表示中の Session と実行中の Workflow の継続。とくに時間上限は、変える部分「明示再走査の終端」の終端条件に既存の client deadline を含めてよいという前提として維持する。理由: Thread 16600fee の `[FIX_POLICY]` が受入条件として明示しているため。
- R-001〜R-016 と B-001〜B-023 の受入条件。human.decisions D-07・D-08 は要求・受入条件を変えないと明記しており、D-05・D-06・D-09・D-10 も観測可能な結果を変えない。理由: 人間がそう明示したため。

## 未確定・リスク

- human.decisions D-06 は「呼び出し元が消えた」command の削除としているが、`list_branches_with_status_snapshot` には生きた呼び出し元が残っている（`src/components/workspace/CreateWorktreeModal.tsx:124`、`src/components/workspace/CreateWorktreeModal.test.tsx:110`）。`list_branches_with_status`（非 snapshot）側は `src/generated` の自動生成分と Rust 側だけで frontend の呼び出し元がない。前提が成立しないまま `list_branches_with_status_snapshot` を削除すると Worktree 作成の branch 候補の取得が失われ、変える部分「Worktree 作成モーダルの入力状態の保持」の受入条件と `requirements.md` Scope / 変更しない対象（Worktree の作成の操作）に反する。D-06 は人間の決定であり、この周では採否を判断していない。
- この周で自動判断した箇所はない。Requirements と Behavior の対応に誤り・不足・矛盾は見つからず、Design では両文書を変更していない。Assumptions に「自動判断」はなく、未決のまま残した要求もない。`[DEFERRED]` で人間へ渡した件はない。
