# Context

- 正本: [#1878 `[01] 状態の変化を購読で届ける土台を作り、Repository 一覧を移す`](https://github.com/siro33950/releash/issues/1878)
- 最初の周の調査基準は branch `feat/issues/1878` の `1ad87359`。
- アーキテクチャの正本は `AGENTS.md`。アプリケーションロジックは Rust が所有し、frontend は表示とレイアウト制御、入力受付、呼び出し、表示用フォーマットだけを担う。
- 現行実装の確認先: `src-tauri/src/adaptor/controller/api/client_service.rs`、`src-tauri/src/infrastructure/push.rs`、`src-tauri/src/adaptor/gateway/push.rs`、`src-tauri/src/adaptor/gateway/repository/notify.rs`、`src-tauri/src/usecase/repo_paths_usecase.rs`、`src-tauri/src/usecase/workspace_tree/list.rs`、`src-tauri/src/usecase/repository_state/{service,worktree}.rs`、`src-tauri/src/domain/repository/watch_subscriptions.rs`、`src-tauri/src/domain/terminal_surface/subscriptions.rs`、`src/lib/client.ts`、`src/hooks/useRepoList.ts`、`src/hooks/useWorkspaceList.ts`、`proto/client.proto`
- 参考にする一般的な作りは、Kubernetes API の list + watch（`resourceVersion`、bookmark、`410 Gone` による再取得: https://kubernetes.io/docs/reference/using-api/api-concepts/ ）と、遅い watch の再開を扱う KEP-6268（ https://github.com/kubernetes/enhancements/issues/6268 ）、および xDS の ADS（1 本の stream に全ての購読対象を載せる）である。
- 同じマイルストーン（[02] UI と daemon の間の通信の仕組みを一本化する）の中で、本 ISSUE は土台を作る回である。各画面の読み取りを購読へ移す作業は #1885（Workspaces）、#1886（Review）、#1887（Automation）、#1888（terminal）が担い、接続を確立する `GetServerInfo` と接続状態の保持は #1895、再接続時に画面を作り直さない扱いは #1896 が担う。
- 正本 Issue が「`get_repo_paths` を Workspaces の Repository 一覧の読み取り」として挙げた前提は、調査基準時点では成立していない。`ac6a539f`（#1884）により `src` から `get_repo_paths` を呼ぶ経路は無くなり、`src/App.restoration.test.tsx:287-291` が呼ばれないことを検証している。Workspaces が表示する Repository 一覧は `get_workspaces` / `refresh_workspaces` の結果（`WorkspaceListSnapshotDto`）から描かれている。
- 正本 Issue が削除対象に挙げた `ListBranchesWithStatus` に対応する rpc は `proto/client.proto` に存在しない（`proto/client.proto:23,197` に `reserved "list_branches_with_status"` があるのみ）。名前の近い `ListBranchesWithStatusSnapshot`（`proto/client.proto:3454`）は存在するが `src/components/workspace/CreateWorktreeModal.tsx:125` から呼ばれており、#1885 が移す対象として挙げられている。

# Outcome

対象者は、Releash の UI を使う利用者と、UI と daemon の間の通信を実装・保守する開発者である。

現在、daemon が持つ状態の変化を UI へ届ける仕組みが、対象ごとに別々に作られている。変更通知には通し番号が無く、共有の列から溢れると全ての画面が一斉に取り直す。terminal だけが別の stream で番号付きの出力を流す。購読の上限と後始末も種類ごとに別にあり、何も流れていない間に接続が生きていることを確かめる手段が無い。各画面が取り直し、失敗時の扱い、再試行を個別に持っている。

変更後は、daemon が持つ状態の変化が一つの購読の仕組みで届く。client ごとに 1 本の stream に全ての購読対象が載り、購読すると現在の状態が届き、区切りの印の後は変更が届く。対象ごとの単調に増える版番号によって、つなぎ直しを最後に受け取った版から再開でき、ある購読の送り待ちが溢れても他の購読と stream は止まらない。何も変わらない間も版番号を載せた印が定期的に届くため、UI は無通信を検出してつなぎ直せる。Repository のパス一覧がこの仕組みで届く最初の対象になり、使われていない読み取りの呼び出しは無くなる。

# Current Behavior

調査基準 `1ad87359` のコードで確認した挙動である。

## 変更通知は「変わった」ことだけを伝え、UI が取り直す

- `subscribe_push` は、購読の開始時に `Resync` を 1 件返し、以後は共有の列から受け取った通知をそのまま流す（`src-tauri/src/adaptor/controller/api/client_service.rs:16-57`）。
- 共有の列は 64 件の `broadcast` であり（`src-tauri/src/infrastructure/push.rs:10`）、溢れると（`Lagged`）購読し直して `Resync` を送る（`client_service.rs:41-48`）。
- 通知の種類は `Push` の `oneof event` に列挙されており、`repo_paths_changed` は Repository のパス一覧そのものを payload に持つ（`proto/client.proto:6-19`）。他の通知は payload を持たないか、対象の識別子だけを持つ。
- UI は `Resync` を受け取ると、登録済みの全ての listener の再取得を一斉に呼ぶ（`src/lib/client.ts:143-146,177`）。通知には通し番号が無く、UI は受け取った通知が連続しているかを判定できない。

## terminal だけが別の stream で番号付きの出力を流す

- `SubscribeTerminalSurfaces` は `Push` とは別の stream であり（`proto/client.proto:3390`）、snapshot と番号付きの出力を流し、UI からの受信確認で送る量を制限する。

## 一覧の版番号は監視状態の中にあり、購読し直すと振り直される

- 一覧の版番号は worktree ごとの監視状態の中にあり、0 から始まる（`src-tauri/src/usecase/repository_state/worktree.rs:113`）。監視の購読者が居なくなると監視状態ごと破棄され、次の購読で 0 から作り直される。
- 監視状態が無いときの読み出しは版番号 0 を返す（`src-tauri/src/usecase/repository_state/service.rs:100-110`）。

## 購読の上限と後始末が種類ごとに別にある

- 変更通知の購読は 16 件まで、その下の監視は合計 64 件までである（`src-tauri/src/domain/repository/watch_subscriptions.rs:42,59-66`）。
- terminal の購読は 16 件まで、attach は 16 件までである（`src-tauri/src/domain/terminal_surface/subscriptions.rs:3-4`）。

## 何も流れていない間の生存確認が無い

- UI は変更通知の stream を期限なしで開き（`src/lib/client.ts:157-159`）、daemon は変更が無い間、何も送らない。無通信と切断を区別する手段が無い。

## UI の各画面が取り直しと再試行を個別に持つ

- `useWorkspaceList` は `workspace-list-changed`、`branch-list-sync`、`repo-paths-changed`、`workflow-execution-changed`、agent session の変化、`window` のイベント、および可視時の定期実行から、それぞれ独立に再取得を起動する（`src/hooks/useWorkspaceList.ts:101-153`）。

## Repository 一覧の現状

- `GetRepoPaths` は `proto/client.proto:3433` に存在するが、`src` の本番コードから呼ばれていない。`src/App.restoration.test.tsx:287-291` が呼ばれないことを検証している。残る呼び出しは、通信経路そのものを確かめる `src/lib/client.test.ts` と `src/lib/client.desktop.test.ts` だけである。`src-tauri/src/cli/` からも呼ばれていない。
- Workspaces が表示する Repository 一覧は、`get_workspaces` / `refresh_workspaces` が返す `WorkspaceListSnapshotDto` の `repositories`（`proto/client.proto:3609-3613`）から描かれる。その元は `RepoPathsUsecase::get()`（`src-tauri/src/usecase/workspace_tree/list_query_service.rs:37-39`、`src-tauri/src/usecase/workspace_tree/list.rs:149-153`）であり、`GetRepoPaths` が返すものと同じ一覧である。同じ一覧が 2 つの経路から出ている。
- `WorkspaceListSnapshotDto` は `generation` を持つ（`proto/client.proto:3610`）。これは Workspaces 一覧の再取得の世代であり、Repository のパス一覧そのものの版番号ではない。
- `repo-paths-changed` は、Repository のパスの追加・削除が成功したときだけ、現在の一覧を payload として発火する（`src-tauri/src/usecase/repo_paths_usecase.rs:31-47`、`src-tauri/src/adaptor/gateway/repository/notify.rs:16-19`、`src-tauri/src/adaptor/gateway/push.rs:61-68`）。
- `repo-paths-changed` を受けているのは `src/hooks/useWorkspaceList.ts:136` だけであり、受け取ると Workspaces 一覧の全体再取得（`refresh_workspaces`）を起動する。Repository の追加・削除が Workspaces の表示へ反映される経路は、これと 可視時の定期実行（既定は 120 秒間隔。`src/hooks/useWorkspaceList.ts:117-121,142-144`）である。
- `workspace-list-changed` は Workspaces 一覧の再取得処理の中からだけ発火する（`src-tauri/src/usecase/workspace_tree/list.rs:187,221`）。Repository のパスの追加・削除では発火しない。
- `useRepoList` は `add_repo_path` と `remove_repo_path` を呼ぶだけであり、一覧の取り直しの処理を持たない（`src/hooks/useRepoList.ts:1-31`）。正本 Issue が削除対象に挙げた「`useRepoList` の取り直しの処理」は、調査基準時点では存在しない。

## 使われていない読み取りの呼び出し

- 正本 Issue が削除対象に挙げた 27 件のうち、`proto/client.proto` に rpc として存在するのは 26 件である（`GetCrashReportingEnabled`、`GetFileAtRef`、`GetStagedContent`、`GetBinaryStagedContent`、`GetFileAtBranchBase`、`GetBinaryFileAtBranchBase`、`GetBinaryFileAtRef`、`GetBranchDiffSummary`、`GetHeadDiffFileTreeSnapshot`、`GetRelativePath`、`GetReviewThread`、`GetReviewThreadHistory`、`GetDefaultBranch`、`GetGitStatus`、`GetGitStatusSnapshot`、`GetStatusDiffStats`、`GetStatusDiffStatsSnapshot`、`GetGitLog`、`GetWorktreeDirtyCount`、`GetRepoGitDir`、`ListWorkflowExecutions`、`GetWorkflowExecution`、`GetWorkflowExecutionLog`、`GetWorkflowNodeDetail`、`ResolveWorktreeByExecution`、`ListFacets`）。この 26 件は `src/generated/` 以下の生成物と `src/lib/client.test.ts` を除いて `src` から呼ばれておらず、`src-tauri/src/cli/` からも呼ばれていない。
- `ListBranchesWithStatus` は rpc として存在せず、`proto/client.proto:23,197` で予約済みである。同名の gateway 関数 `list_branches_with_status`（`src-tauri/src/adaptor/gateway/repository/branch_card.rs:241`）は走査の経路から使われており、この rpc だけが使っているコードではない。

# Scope / Non-goals

## 変更する対象

- daemon が持つ状態の変化を届ける購読の仕組み（daemon 側の配信と、UI 側の受け取り）
- 購読の開始と停止の手段
- 購読の最初の状態、区切りの印、以後の変更の届き方
- 対象ごとの版番号と、つなぎ直しの再開
- 購読ごとの送り待ちと、溢れたときの扱い
- 無通信の検出に使う印の配信
- 購読の重複、存在しない対象の購読、stream 終了時の後始末
- Repository のパス一覧の届き方
- Repository のパス一覧を表示する画面（Settings の Repositories）が一覧を得る経路
- `GetRepoPaths` の削除
- `repo-paths-changed` の削除
- 使われていない読み取りの呼び出し 26 件の削除と、この変更で使われなくなるコードの削除
- 今回変更したファイルの中で使われていないコードの削除

## 変更しない対象

- 接続を確立する `GetServerInfo` と、接続状態の client 側での保持。#1895 が扱う
- Workspaces、Review、Automation、terminal の各画面の読み取りを購読へ移す作業。#1885、#1886、#1887、#1888 がそれぞれ扱う。Workspaces のツリーが表示する一覧（`get_workspaces` / `refresh_workspaces` が返す `WorkspaceListSnapshotDto`）の購読への移設は #1885 が扱う
- 再接続時に画面を作り直さない扱い。#1896 が扱う
- 状態を変える操作と、client の入力に対する計算の呼び出し方。単発の呼び出しのまま変えない
- 双方向の stream への載せ替え。ネイティブ UI（#78）の B13 以降に行う
- 購読の仕組みで差分を送る対象の実装。差分を使うのは terminal だけであり、#1888 が扱う

# Requirements

- R-001: daemon が持つ状態は購読で届く。購読へ移した状態を返す単発の呼び出しは無い。
- R-002: 状態を変える操作と、client の入力に対する計算は単発の呼び出しで行える。
- R-003: 購読の単位は、画面が使う読み取り結果 1 つである。複数の状態の組み合わせは daemon が行い、client は受け取った結果をそのまま表示できる。
- R-004: 購読を開始すると、まず現在の状態が届き、続いて区切りの印が届く。区切りの印より後は、変更だけが届く。
- R-005: 変更の届け方は、対象ごとに対象を丸ごと送るか差分を送るかを選べる。本 ISSUE で購読へ移す対象は丸ごと送る。
- R-006: 一つの client の全ての購読対象は、その client の 1 本の stream で届く。
- R-007: 購読の開始と停止は、単発の呼び出しで行える。
- R-008: 対象ごとに、daemon が単調に増える版番号を付ける。購読で届く状態、変更、印には版番号が付く。単調性は daemon の一回の起動の中で保たれる。
- R-009: つなぎ直すときは、最後に受け取った版を指定して続きから再開できる。指定した版から再開できない場合、および daemon の別の起動で振られた版を指定した場合は、現在の状態が最初から届く。
- R-010: 送り待ちは購読ごとに持つ。ある購読の送り待ちが溢れた場合、その購読だけが版からの再開になる。同じ stream の他の購読と、stream そのものは止まらない。
- R-011: 変更が無い間も、版番号を載せた印が定期的に届く。
- R-012: 一つの client が持てる購読の数に上限を設けない。
- R-013: 同じ stream の中で同じ対象を重ねて購読しても、その対象の購読は 1 件にまとまる。
- R-014: 存在しない対象の購読は拒否される。
- R-015: stream が終わると、その client の購読は全て終わる。
- R-016: Repository のパス一覧は購読で届く。
- R-017: Repository のパス一覧を返す単発の呼び出し（`GetRepoPaths`）は無い。
- R-018: `repo-paths-changed` の変更通知は無い。
- R-019: Repository の追加・削除の後も、Workspaces は追加・削除の結果を反映した Repository 一覧を表示する。
- R-020: `src` から使われていない読み取りの呼び出しは無い。対象は `GetCrashReportingEnabled`、`GetFileAtRef`、`GetStagedContent`、`GetBinaryStagedContent`、`GetFileAtBranchBase`、`GetBinaryFileAtBranchBase`、`GetBinaryFileAtRef`、`GetBranchDiffSummary`、`GetHeadDiffFileTreeSnapshot`、`GetRelativePath`、`GetReviewThread`、`GetReviewThreadHistory`、`GetDefaultBranch`、`GetGitStatus`、`GetGitStatusSnapshot`、`GetStatusDiffStats`、`GetStatusDiffStatsSnapshot`、`GetGitLog`、`GetWorktreeDirtyCount`、`GetRepoGitDir`、`ListWorkflowExecutions`、`GetWorkflowExecution`、`GetWorkflowExecutionLog`、`GetWorkflowNodeDetail`、`ResolveWorktreeByExecution`、`ListFacets` である。
- R-021: この変更で使われなくなるコードと、この変更で触れたファイルの中で使われていないコードは無い。対象は Rust、TypeScript、proto、およびそれらだけを対象とするテストである。

# Assumptions / Open Questions

なし。
