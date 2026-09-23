# Context

- 正本: [#1861 `Workspaces の更新中・更新失敗時に一覧を保持し、全体更新ボタンから再取得できるようにする`](https://github.com/siro33950/releash/issues/1861)
- 最初の周の調査基準は branch `feat/issues/1861` の `81ec380b`。正本 Issue のコード参照は `b642f94d` 時点の行範囲で示されており、`src/components/workspace/WorkspaceList.tsx` はその後に変更されているため、本文書の参照は `81ec380b` の位置で示す
- アーキテクチャの正本: `AGENTS.md`。アプリケーションロジックは Rust が所有し、frontend は表示とレイアウト制御、入力受付、`invoke` の呼び出し、表示用フォーマットだけを担う
- 現行実装の確認先: `src/hooks/useRepoList.ts`、`src/hooks/useWorktreeList.ts`、`src/hooks/useWorkspaceTreeNodes.ts`、`src/components/workspace/WorkspaceList.tsx`、`src-tauri/src/usecase/repository_state/{service,worker,worktree,snapshot}.rs`
- 正本 Issue は、発生事象（Workspaces の Worktree 名がすべて消え、エラーメッセージだけが表示され、アプリの再起動まで戻らなかった）について、エラーの文言と直接原因が未確定であることを明記している。Issue が挙げる「確認できた問題」4点は、その調査でコード上確認できた問題であり、当日の直接原因と断定されていない
- Workspaces の一覧は、登録 Repository 一覧、Repository ごとの Worktree 一覧、Worktree 配下の Session・Workflow 一覧の3階層で構成され、取得経路がそれぞれ独立している
- 登録 Repository 一覧は Workspaces と設定画面の両方が表示する。`81ec380b` では両者が同じ `useRepoList` の結果を参照していた（`src/App.tsx:113-119`、`src/App.tsx:289`、`src/App.tsx:332`）

# Outcome

対象者は、Releash の Workspaces から Repository・Worktree・Session・Workflow を辿って作業対象を選ぶ利用者である。

現在、Workspaces の一覧は更新の開始と更新の失敗の両方で失われる。Repository ごとの Refresh を押すと Repository 配下の表示が取得完了まで差し替わり、配下の Worktree・Session・Workflow の一覧と展開状態が破棄される。自動更新では、未取得を表す空の結果をそのまま一覧として反映しうる。走査の失敗は画面に現れず、失敗した状態からアプリを再起動せずに再取得する操作もない。

変更後は、更新中も更新に失敗した後も、最後に取得できた一覧を表示し続ける。Workspaces 行に置いた一つの更新操作から、折りたたまれた Repository を含む一覧全体を実際に再取得でき、失敗した範囲は前回の情報とともに失敗として示され、再取得に成功すれば解消する。

# Current Behavior

`81ec380b` のコードで確認した挙動である。

## 更新の開始で配下の表示が破棄される

- `useWorktreeList.refresh()` は `silent` 指定がない場合に `loading` を立てる（`src/hooks/useWorktreeList.ts:46-73`）。
- `RepoTreeSectionView` は `loading` の間、Repository 配下の一覧をスピナー1つへ差し替える（`src/components/workspace/WorkspaceList.tsx:1660-1684`）。配下の `WorktreeTreeItem` が unmount され、そこに紐づく展開状態と `useWorkspaceTreeNodes` が保持する Session・Workflow 一覧が失われる。
- 再現手順は、Repository を展開し、配下の Worktree を展開して Session・Workflow を表示した状態で、その Repository 行の Refresh（`aria-label` は `Refresh <repoName>`、`src/components/workspace/WorkspaceList.tsx:1648-1658`）を押すことである。

## 未取得を表す空の結果を一覧として反映しうる

- 自動更新は `silent: true` で呼ばれ、スピナーへの差し替えは行わない（`src/hooks/useWorktreeList.ts:90-109`）。
- `useWorktreeList.refresh()` は取得した snapshot の `loading`・`stale` を参照せず `worktree_display_groups` を反映する（`src/hooks/useWorktreeList.ts:51-63`）。
- `RepositorySnapshot::loading()` は `version: 0`、`flags.loading: true`、`branch_cards` 空の snapshot であり（`src-tauri/src/usecase/repository_state/snapshot.rs:44-56`）、走査が完了していない状態でも読み出されうる。

## 手動更新が実際の再走査を要求しない

- `RepositoryStateService::get_snapshot` は監視中の worktree について保存済み snapshot をそのまま返し、走査を再実行しない（`src-tauri/src/usecase/repository_state/service.rs:93-105`）。
- 走査の再実行は `WorktreeState::invalidate`（`src-tauri/src/usecase/repository_state/worktree.rs:190-201`）経由であり、file watcher / git watcher の通知からのみ呼ばれる。frontend から再走査を要求する経路はない。

## 更新失敗が画面に現れない

- Repository の走査失敗は `log::warn!` と `state.mark_scan_failed()` だけで、frontend へ通知しない（`src-tauri/src/usecase/repository_state/worker.rs:119-125`）。
- `mark_scan_failed` は `version == 0` かつ `loading` の snapshot について `loading` を下ろす（`src-tauri/src/usecase/repository_state/worktree.rs:233-239`）。初回走査に失敗した Repository は、空で取得完了した snapshot として読み出される。
- `useWorktreeList.refresh()` の取得失敗は `console.error` のみで、画面に失敗を示さない（`src/hooks/useWorktreeList.ts:64-65`）。
- Worktree 配下の Session・Workflow 一覧は、取得に失敗しても読み込み済みであれば前回の項目を保持する（`src/hooks/useWorkspaceTreeNodes.ts:184-196`）。一方、その失敗は `nodes.length === 0` のときだけ表示され、既存項目がある場合は表示されない（`src/components/workspace/WorkspaceList.tsx:1429-1436`）。

## 全体を更新する操作がない

- Workspaces 行にあるのは Add Worktree のボタンだけで、更新の操作はない（`src/components/workspace/WorkspaceList.tsx:1741-1757`）。
- 更新の操作は Repository 行ごとに存在する（`src/components/workspace/WorkspaceList.tsx:1648-1658`）。押した Repository の Worktree 一覧だけが対象で、折りたたまれた Repository や配下の Session・Workflow は対象にならない。
- 登録 Repository 一覧は起動時の `get_repo_paths` と `repo-paths-changed` イベントでのみ更新され、再取得の操作はない（`src/hooks/useRepoList.ts:19-50`）。起動時の取得失敗は復元失敗として扱われる（`src/App.tsx:126-129`）。
- Worktree 配下の Session・Workflow 一覧は、`WorktreeTreeItem` の mount と Worktree 単位のイベントで取得される（`src/hooks/useWorkspaceTreeNodes.ts:276-320`）。表示部品の再作成に依存しない更新経路はない。

# Scope / Non-goals

## 変更する対象

- 自動更新・手動更新の開始時、および更新失敗時の、Repository・Worktree・Session・Workflow の一覧表示の保持
- 更新をまたいだ展開状態、選択、Workspaces 一覧のスクロール位置、表示中 Session の維持
- Workspaces 行への一つの更新操作の追加と、Repository ごとの更新操作の撤去
- 登録 Repository 一覧、Worktree 一覧、Session・Workflow 一覧を対象とする、表示部品の再作成に依存しない更新
- 手動更新による Repository の走査の再実行
- 更新失敗の画面への表示と、再取得成功による解消
- 初回取得中、初回取得失敗、正常に取得した空の結果の区別
- 手動更新の進行表示と重複操作の抑止
- 更新結果の新旧の判定
- 登録 Repository 一覧を表示する画面が参照する一覧の、Workspaces の更新結果への一本化

## 変更しない対象

- 正本 Issue が記録した当日の事象の直接原因の特定と、その原因への個別の対処。Issue は直接原因を未確定としており、完了条件にも含めていない
- 自動更新の周期と起動契機。対象範囲は変えるが、自動更新が始まる周期と契機そのものは変えない
- Session の内容、Workflow の実行そのものの取得・再実行。更新対象は Workspaces の一覧情報に限る
- Repository の追加・削除、Worktree の作成・削除の操作
- Workspaces 以外の画面の更新経路。ただし登録 Repository 一覧が参照する一覧は R-016 の対象とする
- Workspaces 以外の画面での取得状態の区別。R-010 の初回取得中・初回取得の失敗・正常に取得した空の結果の区別は Workspaces に限る
- 手動更新に固有の時間上限の導入。応答が返らない場合の打ち切りは、既存の client の deadline に委ねる

# Requirements

- R-001: 自動更新・手動更新のいずれでも、更新の開始を理由に Repository・Worktree・Session・Workflow の一覧表示を消さない。更新中は直前に取得できた一覧を表示し続ける。
- R-002: 更新の前後で存在し続ける対象について、更新をまたいで展開状態、選択、Workspaces 一覧のスクロール位置を維持し、表示中の Session を切り替えない。
- R-003: Workspaces 行に更新の操作を一つ置く。Repository ごとの更新の操作はない。
- R-004: 一覧の取得に失敗している状態でも、Workspaces 行の更新の操作を実行できる。
- R-005: 手動更新と自動更新は、登録 Repository 一覧、各 Repository の Worktree 一覧、各 Worktree 配下の Session・Workflow の一覧情報を対象とする。折りたたまれている Repository も対象に含み、表示部品の再作成に依存せずに更新する。
- R-006: 手動更新と自動更新は、Repository について走査を実際に再実行し、その結果を一覧へ反映する。保存済みの結果をそのまま反映しない。
- R-007: 自動更新に失敗した後も、アプリを再起動せず Workspaces 行の更新の操作から再取得でき、取得に成功した対象は最新の内容で表示される。
- R-008: 一部または全部の対象で取得に失敗しても、取得に成功した対象は更新し、失敗した対象は直前に取得できた一覧を表示し続ける。
- R-009: 取得に失敗した Repository・Worktree には、その一覧の更新に失敗したことと、表示しているのが前回取得の情報であることを示す。登録 Repository 一覧そのものの取得に失敗した場合は、Workspaces 一覧全体に同じ内容を示す。
- R-010: 初回取得中、初回取得の失敗、正常に取得した空の結果を区別して示す。初回取得中と初回取得の失敗を「項目なし」として表示しない。
- R-011: 再取得に成功した対象では、それまでの更新失敗の表示を解消する。
- R-012: 手動更新の進行中は、進行していることを示し、同じ更新の重複した要求を受け付けない。更新が成功・失敗のいずれで終わった後も、再び操作できる。
- R-013: 自動更新と手動更新が重なり応答の順序が入れ替わっても、より古い取得結果でより新しい一覧を上書きしない。
- R-014: 正常に取得した結果が空である場合、および正常な取得によって対象の削除が確認できた場合は、その結果を一覧へ反映する。
- R-015: 更新中、更新の失敗中、失敗からの復旧後を通して、表示中の Session と実行中の Workflow の実行は継続する。
- R-016: 登録 Repository 一覧を表示する画面は、Workspaces が表示している登録 Repository 一覧と同じ Repository を表示する。Workspaces の更新によって登録 Repository 一覧が変わった場合も一致する。

# Assumptions / Open Questions

なし。
