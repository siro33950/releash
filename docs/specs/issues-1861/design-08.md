# Design 08

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `feat/issues/1861` の `81ec380b`。Design 01〜06 の周の実装は `8eff7b47`（`fix(workspace): 更新中の一覧保持と全体再取得を実装 (#1861)`）としてコミット済みで、Design 07 の周の実装は作業ツリーの未コミット変更（`proto/`、`src-tauri/`、`src/`、`tests/` の変更群）として存在する。Spec 工程ではコードを変更していないため、この未コミット分を含む作業ツリーをこの周の開始状態とする。
- 直前の Design は `docs/specs/issues-1861/design-07.md`（入力の `context.design_path` と一致）。
- Design 07 の周で扱った open Thread 6件（35a998c2-07b9-44c4-ade5-37fc5069d579、ea71ac18-5360-4527-bcef-0b8abe6bf34f、7d95e1ef-e703-43f6-81a0-d4e808e175fb、bd89e8ce-d93f-4a20-84a2-88585aa0ef61、16600fee-8ad7-471c-a828-e795190e14d9、1cdbeeec-41dd-4651-a790-b995b13d22e7）は開始時点でいずれも resolved であり、open ではない。
- 開始時点の open Thread は4件（f69dc569-5cc1-4bc7-8a8f-bea921311ba3、7e2110e0-c833-45f2-9f0f-1c86baa2d229、53ba0525-22d3-4783-8348-3acb15fde880、cae4610d-2107-4e7d-aa3f-27be2ce98a90）で、いずれも `[FIX_POLICY]` が付いている。`[REJECTED]`・`[DEFERRED]` にした Thread はない。
- Requirements・Behavior はこの周の前段（triage_requirements_and_behavior）で更新しておらず、Design でも変更していない。R-001〜R-020 と B-001〜B-027 に欠番・重複はなく、対応表は全 Requirement ID を記載して各行に Behavior ID が対応している。Assumptions / Open Questions は「なし」である。
- Design 07 の「未確定・リスク」に挙げた human.decisions D-06 の前提不成立は、command 登録から `list_branches_with_status`（非 snapshot）だけを外し、生きた呼び出し元がある `list_branches_with_status_snapshot`（`src/components/workspace/CreateWorktreeModal.tsx:125`）を残す形で開始状態に反映されている。ほかに Design 07 から引き継ぐ未確定事項はない。

## 変える部分

- `BackendPush::WorkspaceListChanged` の変換の境界テスト: push gateway の変換を gateway 境界のテストで検証する。開始状態では `src-tauri/src/adaptor/gateway/push.rs:47-51` が `BackendPush::WorkspaceListChanged` を event 名 `workspace-list-changed` と `wire::push::Event::WorkspaceListChanged` へ結線する唯一の変換であり、`src-tauri/src/adaptor/gateway/push_test.rs` は購読 frame 制御（同:4）と `AgentSessionChanged`（同:28）だけを検証し、`src-tauri/src/adaptor/protocol/client/client_test.rs` にもこの variant の往復ケースがない。根拠: Thread f69dc569-5cc1-4bc7-8a8f-bea921311ba3（`[FIX_POLICY]`、要旨は adaptor/gateway のモデル変換テストを必須とする `docs/architecture/TEST.md` に対し、R-017 の PR 情報の反映通知が経由するこの変換に境界テストがない）。ルート: 委任（テストの置き場所と書き方は `docs/architecture/TEST.md` の配置・命名規則に従う）
- `WorkspaceListRefresh` の domain 境界と公開説明の一致: 一覧更新の状態所有者がどの domain 境界に属するかを、その境界の公開説明から判断できるようにする。開始状態では `src-tauri/src/domain/workspace_tree/mod.rs:1-4` が境界を「The aggregate is restored from canonical execution/node/session records. It deliberately has no dedicated snapshot, revision, or CAS lifecycle.」と宣言する一方、同 module の `src-tauri/src/domain/workspace_tree/refresh.rs:85-97` の `WorkspaceListRefresh` が `generation`・`snapshot_generation` を保持し、同:106 の `begin` と generation 照合付きの `complete_repositories` / `complete_branches` / `complete_worktree` で一覧更新の lifecycle を所有している。根拠: Thread 7e2110e0-c833-45f2-9f0f-1c86baa2d229（`[FIX_POLICY]`、要旨は `docs/architecture/DOMAIN.md`「配置は『どのドメインに凝集するか』で決める」「モジュール公開インターフェース」に対し宣言した境界と module の内容が食い違っている事実）。ルート: 委任（型の移設と境界記述の改訂のどちらで一致させるかを含む）
- アーカイブ後の再照合での Workflow 履歴の更新: 選択中の Workflow をアーカイブした直後に、履歴の表示をアーカイブ後の一覧に揃える。開始状態では `src/hooks/useWorkspaceTreeNodes.ts:141-179` の再照合経路が `get_workspace_tree_selection_reconciliation` の snapshot だけを取得して既存 `treeState` を spread するため `workflowHistory` を更新せず、`workflowHistory` の更新経路は同:284-289 の list（`refresh_workspaces` / `get_workspaces` の結果）経由だけである。`archive_workspace_workflow_execution`（`src-tauri/src/usecase/workflow/workspace_tree.rs:257-265`）はこの list を更新する通知を出さない。根拠: Thread 53ba0525-22d3-4783-8348-3acb15fde880（blocking、`[FIX_POLICY]`）、R-005「手動更新と自動更新は、登録 Repository 一覧、各 Repository の Worktree 一覧、各 Worktree 配下の Session・Workflow の一覧情報、および各 Repository の PR 情報を対象とする」、`requirements.md` Scope / 変更しない対象「Session の内容、Workflow の実行そのものの取得・再実行」。ルート: 委任（再照合結果に履歴を含めるか、一覧の更新通知を出すか等）
- 未開始の全体更新の要求の統合の所有者: 同じ `WorkspaceListUsecase` を使う経路であれば、renderer を介するか否かにかかわらず未開始の全体更新の要求が一つに統合されるようにする。開始状態では `src-tauri/src/usecase/workspace_tree/list.rs:100-117` の `refresh` が呼出しごとに `WorkspaceListRefresh::begin` を実行するだけで統合の状態を持たず、統合は `src/hooks/useWorkspaceList.ts:43-57` の `pendingFullRequestRef` / `requestsRef` にしかない。`refresh_workspaces` は Tauri と local API が共有する command router（`src-tauri/src/adaptor/controller/client/workspace_tree_shared.rs:32`）に登録されている。根拠: Thread cae4610d-2107-4e7d-aa3f-27be2ce98a90（blocking、`[FIX_POLICY]`）、R-020「更新の処理中に同じ全体更新の契機が複数回生じても、まだ開始していない全体更新の要求は一つに統合される」、B-027。ルート: 置き場所は Design 01 固定ルート①により Rust 側。統合状態の持ち方、進行中の要求との関係、snapshot の返し方は委任

## 固定するルート

- Design 01 で固定した次の4つを維持する（human.constraints）。①全体の更新手順と結果の判定を Rust 側に置き、既存の `RepositoryStateService` の走査処理と Workspace の一覧取得処理を再利用する。Frontend は受け取った一覧・更新状態・エラーを表示するだけにする。②一覧を保持する変更と、Session・Workflow の再取得経路の変更を併せて行う。③更新対象は Workspaces の一覧情報に限り、表示中の Session や実行中の Workflow の継続を保つ。④手動更新と自動更新で内部の経路を分けない。
- Design 07 で固定した「PR 情報の分離の実施場所」を維持する。一覧の取得と PR 情報の取得の分離は Rust 側で行い、派生点 `81ec380b` のように frontend で PR 情報を重ね直す形へは戻さない（human.decisions D-01）。
- この周で新しく固定する実装上の指定はない。変える部分「未開始の全体更新の要求の統合の所有者」で置き場所が Rust 側であることは、上記 Design 01 固定ルート①からの帰結であり、新しい指定ではない。

## 変えないもの

- Design 01 の「変えないもの」3件を維持する。自動更新の周期と起動契機、手動更新に固有の時間上限を設けず応答が返らない場合の打ち切りを既存 client の deadline に委ねること、表示中の Session と実行中の Workflow の継続。
- R-001「自動更新・手動更新のいずれでも、更新の開始を理由に一覧表示を消さない」、R-013「より古い取得結果でより新しい一覧を上書きしない」、B-020「登録 Repository 一覧の取得失敗の表示」、および `requirements.md` の Non-goals（human.constraints）。理由: 人間がこれらを維持対象として示しているため。
- R-001〜R-020 と B-001〜B-027 が定める観測可能な結果。今周の4件はいずれも要求・受入条件を増減させない。とくに Thread 7e2110e0 の `[FIX_POLICY]` は「Requirements・Behavior に観測可能な変更を加えない」を明示している。
- 実装量を理由に今周の対象範囲を縮小しないこと、および構造は単純な形を優先すること（human.constraints）。

## 未確定・リスク

- この周で自動判断した箇所はない。Requirements と Behavior の対応に誤り・不足・矛盾は見つからず、Design では両文書を変更していない。Assumptions に「自動判断」はなく、未決のまま残した要求もない。`[DEFERRED]` で人間へ渡した件もない。
