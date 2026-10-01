# Design

## 変える部分
- 購読中の読み直しの失敗を画面へ送る: 読み直し・外部の情報の取り直し・端末の表示の取り直しの失敗を、ログだけで終えずに購読の stream で画面へ送る（`usecase/state_subscription.rs:270-276,288,494-497`、proto の購読のイベント、presenter、`src/lib/client.ts`）。根拠: R-002「購読を始めた後に状態を読み直して失敗した場合も、読めなかったことが画面に届く」。ルート: 委任
- 最初の読み取りに失敗した購読を読み直す: 最初の読み取りに失敗しても、対象の状態の変化で読み直す（`usecase/state_subscription.rs:202-203`）。根拠: R-003「最初の読み取りに失敗した購読も、対象の状態が変われば読み直され」。ルート: 委任
- 画面側で失敗を対象ごとに覚える: 後から同じ対象を購読した画面にも失敗を渡す（`src/lib/client.ts:416-421`）。根拠: R-004「読めていない対象を後から別の画面が購読したとき、読めないことがその画面にも届く」。ルート: 委任
- 購読の失敗を受け取らない書き方をなくす: `subscribeState` の失敗のコールバック（`src/lib/client.ts:397-401`）を必須にする。値だけを取り出す購読（`src/hooks/useStateSubscription.ts:35-39`）の呼び出し元 8 か所と、購読を直接呼ぶ 4 か所（`useRepoList.ts:14`、`CreateWorktreeModal.tsx:106-117,118-121`、`useWorkspaceTreeNodes.ts:32`）は、失敗を受け取り、値を使う場所で表示する。根拠: R-017「次の画面は、購読の失敗を受け取って表示する」。ルート: 下の「固定するルート」の 1
- workflow の定義の読み取り: `get_workflow_dto`（`usecase/workflow/mod.rs:344-357`）は、読み込めないときと形式を読めないときに失敗を返す。形式の既定値（`:355`）もこの中で一緒に直す。根拠: R-005「読み込めない workflow の定義は、定義が無いことと区別して画面に届く」。ルート: 委任
- git の変更の状態の走査の失敗を記録する: 走査の失敗を、最後の走査の結果として記録し、読む側に返す（`usecase/repository_state/worktree.rs:153-160,281-287`）。Review の差分・Review のファイルの表示・Workspaces の未コミット数は、それを失敗として受け取る。根拠: R-006、R-007。ルート: 下の「固定するルート」の 2
- Review の差分の購読の hook: `src/hooks/useReviewSnapshot.ts` は失敗を呼び出し元に返し、Review の画面（`src/components/panels/ReviewPanel.tsx`）は「No changes」と区別して表示する。根拠: R-006「Review の画面は差分を読めないことを表示し、『変更が無い』と区別する」。ルート: 委任
- Workspaces の一覧: 一覧を集める処理の失敗（`usecase/workspace_tree/list.rs:76-83`）を空の一覧にせず、未コミット数（`:109-111`）と PR の状態（`:121,147-151`）を読めないときは、それぞれ読めないことを値に残す。根拠: R-007、R-008。ルート: 委任
- ワークスペースの保存された状態: ファイルが無いとき（`adaptor/gateway/workspace_state/repository_impl.rs:44-46`）だけを「無い」とし、読めない・壊れているとき（`:48-51`）は失敗を返す。画面側（`src/hooks/useWorkspaceStateCache.ts:47-58`）は、失敗を「無い」と区別し、読めなかったときは保存しない。根拠: R-009。ルート: 下の「固定するルート」の 3
- Issue の一覧: `gh` の技術的でない失敗（`adaptor/gateway/git_host/github.rs:122-125`）と、出力の解析の失敗（`:238-242`）を失敗として返す。origin の URL を読めないとき（`adaptor/gateway/git_host/discovery.rs:14`）も失敗として返す。根拠: R-010。ルート: 委任
- git の読み取りの「無い」を限る: `git_operation::optional`（`adaptor/gateway/shared/git_operation.rs:84-92`）が None にするのを git2 の NotFound だけにし、呼び出し元 39 か所（`repository/git_config.rs` 12、`repository/worktree.rs` 16、`code/diff_compute.rs` 4、`repository/status.rs` 3、`repository/util.rs` 2、`git_host/discovery.rs` 2）を全て見直す。根拠: R-011、R-012。ルート: 下の「固定するルート」の 4
- 既定ブランチの探索: `infrastructure/git/helpers.rs:22-44` は、参照が無いことと、読み取りの失敗を区別する。根拠: R-011「既定ブランチを探すときの失敗も、『既定ブランチが無い』と区別する」。ルート: 委任
- worktree の一覧の情報: 名前の読み取り（`adaptor/gateway/repository/worktree.rs:114-122`）、lock の状態（`:285`）、ブランチ（`:287-291`）を読めないときは失敗として扱う。根拠: R-013。ルート: 委任
- session の履歴: タイトルと最初の入力の読み取りの失敗（`adaptor/gateway/agent_session/agent_session_history_gateway.rs:77-124`）を、タイトルが無いことと区別して返す。根拠: R-014。ルート: 委任
- provider hook の警告: 壊れた記録（`adaptor/gateway/provider_lifecycle/hook_health_failure_query_impl.rs:45,59`）と、NotFound 以外で情報を読めない記録（`infrastructure/provider_lifecycle/health_marker.rs:89-91`）を読み飛ばさず、失敗として返す。根拠: R-015。ルート: 委任
- workflow の定義ファイルの数え上げを一つにする: 一覧（`adaptor/gateway/workflow/storage.rs:320-345`）と診断（`adaptor/gateway/workflow/diagnostics.rs:1721-1730`）の数え上げを一か所にする。根拠: R-016「一覧と診断は、同じ定義ファイルを対象にする」。ルート: 下の「固定するルート」の 5
- workflow の一覧・facet・診断の失敗: 読めない定義ファイル（`storage.rs:342-344`）を一覧から消さない。facet は、権限の失敗で builtin に切り替えず（`adaptor/gateway/workflow/facet.rs:132`）、読めない説明を空にしない（`:240-244`）。診断は、facet の一覧を読めないとき（`diagnostics.rs:1521,1603`）に失敗を返す。根拠: R-016。ルート: 委任
- 起動時のリポジトリとリポジトリの追加: `src/App.tsx:151-168,176-184` は、リポジトリの外・リポジトリではないことと、読み取りの失敗を区別し、失敗を表示する。根拠: R-018。ルート: 委任
- worktree の一覧から使われていない値を除く: `WorktreeEntryDto` の未コミット数と base ブランチ、それを作る読み取り（`usecase/repository_usecase.rs:131-138` の未コミット数と base ブランチの読み取り、`:265-273`）、proto の `WorktreeEntryDto` の該当フィールドを削除する。根拠: R-019。ルート: 委任

## 固定するルート
1. `subscribeState` の失敗のコールバック（`src/lib/client.ts:397-401`）を必須にし、失敗を黙って捨てる呼び出しを書けなくする。
2. git の変更の状態の走査は、worktree の並びの走査（`usecase/repository_state/worktree.rs:117-138` の `scan_worktrees_once`）と同じく、最後の走査の結果を成功か失敗のまま記録し、読む側に返す。
3. ワークスペースの保存された状態は、ファイルが無いときだけを「無い」とする。読めない・壊れているときは失敗として返す。画面は、読めなかったときは保存しない。
4. `git_operation::optional` 自体を変え、git2 の NotFound だけを None にする。NotFound 以外で「無い」として扱うものは、その呼び出し元で git2 のエラーコードを明示して扱う。明示して扱うのは次の箇所だけで、それ以外は失敗として返す。
   - `repository/git_config.rs:169` の `repo.head()` の UnbornBranch（まだコミットの無いブランチ）: None
   - `repository/worktree.rs:137,280` の `wt.validate()` の失敗（実体の無くなった worktree）: 一覧から外し、一覧全体を失敗にしない
   - `repository/git_config.rs:165` の `client::open` の「リポジトリではない」: None
5. workflow の定義ファイルの数え上げ（`read_dir`、`workflow_source_format` による判定、`file_stem` による名前）を一か所にし、ディレクトリを読めないときは失敗として返す。一覧（`storage.rs:320-345`）と診断（`diagnostics.rs:1721-1730`）はその結果を使う。1 ファイルごとの仕事（一覧は読み込んで要約、診断は中身を読んで診断）は、それぞれに残す。

## 変えないもの
- `src/hooks/useNotionSettings.ts:90`。購読の失敗を受け取って表示しているため。

## 未確定・リスク
- `repository/worktree.rs:244` の `repo.merge_base`: 共通の祖先が無いとき、libgit2 は `GIT_ENOTFOUND` を返す（libgit2-sys-0.18.8+1.9.7 の `merge.c:118-119,250-251`）。NotFound なので、`optional` を変えた後も None のままである。
- `repository/worktree.rs:191` の `commit.parent_id(0)`: 範囲の外では `Error::from_str`（NotFound ではない）を返す（git2-0.21.0 の `commit.rs:311-320`）。ただし直前の `:188` で `parent_count() == 0` なら抜けているので、範囲の外にはならない。ここでの失敗は、本当の失敗として返してよい。
- どちらも、「固定するルート」の 4 に明示の扱いを足す必要は無い。
