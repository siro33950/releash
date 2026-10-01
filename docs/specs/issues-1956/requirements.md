# Context

- 要求の正本: https://github.com/siro33950/releash/issues/1956
- 補助資料: #1886（Review の表示を購読に移す）、#1887（Automation の表示を購読に移す）、#1954（Output Boundary を一方向にする）、#1955（購読の出力の型）、`docs/architecture/README.md`、`docs/architecture/USECASE.md`
- ISSUE の本文は main `636bb2f0` 時点の記述である。この要求は main `7ccaf34e` のコードで確かめた事実に基づく。
- ISSUE の本文が根拠に挙げる「失敗も Output Data として表す」は、今の規約に無い。根拠は `docs/architecture/USECASE.md:57-62`（失敗は戻り値で返す。失敗を文字列へ変換して落とさない）である。
- 処理の重複の扱いは `docs/architecture/README.md:59`（同じ操作の実装は 1 つに集約する）に従う。
- 画面は daemon の状態を購読で受け取る。購読の最初の読み取りの失敗は、購読を始める呼び出しの失敗として画面に届く。購読の stream が送るのは ready・snapshot・change・bookmark だけで、失敗を送る種類は無い（`proto/client.proto:2901-2906`）。

# Outcome

- 対象者: Releash の画面で、workflow の定義、Review の差分、Workspaces の一覧、Issue、ブランチ、session の履歴などを見て判断する開発者。
- 今の問題: daemon が状態を読めなかったとき、画面は「無い」「空」「変更が無い」と同じ表示になる。利用者は、差分が無い、定義が無い、Issue が無い、未設定である、と誤って判断しうる。保存されたワークスペースの状態を読めなかったときは、既定の配置で開き、読めなかったファイルを次の保存で上書きする。
- 変更後の状態: 状態を読めなかったことが、状態が無い・空であることと区別して画面に届く。購読を始めた後に読めなくなった場合も同じく届き、古い値を今の値として出し続けない。読めなかった状態を上書きしない。

# Current Behavior

main `7ccaf34e` のコードを読んで確かめた挙動である。

## 購読の仕組み

- 購読の最初の読み取りに失敗すると、購読を始める呼び出しが失敗し、購読は登録されない（`src-tauri/src/usecase/state_subscription.rs:202-203`）。画面は失敗を受け取る（`src/lib/client.ts:313-320`）が、登録されていないので、後で状態が変わっても読み直されない。
- 購読を始めた後、変化を受けて読み直したときの失敗は、daemon がログに出すだけで画面に送らない（`usecase/state_subscription.rs:288`）。外部の情報の取り直し（Issue の取得）の失敗も、ログに出すだけである（同 `:270-276`）。画面は古い値を出し続ける。
- 端末の表示の取り直しに失敗すると、daemon はログに出して取り直しをやめる（`usecase/state_subscription.rs:494-497`）。
- 画面側で、最初の読み取りに失敗した対象を後から別の画面が購読すると、値も失敗も届かない。失敗を対象ごとに覚えていないためである（`src/lib/client.ts:416-421`）。

## 読み取りで失敗を「無い・空」に変えている箇所

| 対象 | 箇所 | 失敗したときの値 |
|---|---|---|
| workflow の定義 | `usecase/workflow/mod.rs:344-357` | 読み込めない定義を、ログに出して「定義が無い」（None）にする。形式を読めないときは Yaml として扱う |
| Review の差分 | `usecase/repository_state/worktree.rs:153-160,281-287`、`usecase/review_usecase.rs:184-192` | git の変更の状態の走査に失敗すると、ログに出す。まだ一度も読めていなければ、空の snapshot を「読み込み中でも古くもない」値にする。一度読めていれば、前の snapshot を古くない値のまま残す。どちらも正常な値として返す |
| Workspaces の未コミット数 | `usecase/workspace_tree/list.rs:109-111` | 読めないと 0 になる |
| Workspaces の一覧 | `usecase/workspace_tree/list.rs:76-83` | 一覧を集める処理の失敗が、空の一覧になる |
| Workspaces の PR | `usecase/workspace_tree/list.rs:147-151` | PR の取得の失敗をログに出すだけで、「PR が無い」と同じになる |
| ワークスペースの保存された状態 | `adaptor/gateway/workspace_state/repository_impl.rs:41-51` | 読めない・壊れたファイルが、「保存された状態が無い」になる |
| Issue | `adaptor/gateway/git_host/github.rs:122-125,238-242`、`adaptor/gateway/git_host/discovery.rs:14` | `gh` の技術的でない失敗（未認証、API のエラーなど）と、出力を解析できないときは空の一覧になる。origin の URL を読めないときは「GitHub ではない」になる |
| git の読み取り全般（branch base、releash base、worktree の一覧、Review の走査、差分など） | `adaptor/gateway/shared/git_operation.rs:84-92` とその呼び出し元 39 か所 | git2 のどのエラーも「無い」（None）になる |
| 既定ブランチ | `infrastructure/git/helpers.rs:22-44` | 参照を解決するときの失敗が、すべて「既定ブランチが無い」になる |
| worktree の一覧 | `adaptor/gateway/repository/worktree.rs:114-122,285,287-291` | 名前の読み取りに失敗した行が消える。lock の状態を読めないと「lock されていない」になる。worktree を開けないとブランチが `"unknown"` になる |
| session の履歴 | `adaptor/gateway/agent_session/agent_session_history_gateway.rs:77-124` | タイトルと最初の入力を読めないと、「タイトルが無い」になる |
| provider hook の警告 | `adaptor/gateway/provider_lifecycle/hook_health_failure_query_impl.rs:45,59`、`infrastructure/provider_lifecycle/health_marker.rs:89-91` | 壊れた記録と、「無い」以外の理由で情報を読めない記録を読み飛ばし、警告が出ない |
| workflow の一覧 | `adaptor/gateway/workflow/storage.rs:342-344` | 読めない定義ファイルが一覧から消える |
| facet | `adaptor/gateway/workflow/facet.rs:132,240-244` | 権限がなく読めないと、builtin の内容が出る。読めない facet の説明が空になる |
| 診断 | `adaptor/gateway/workflow/diagnostics.rs:1521,1603,1723-1724` | 置き場所や facet の一覧を読めないと、その分の診断が出ず、問題が無いように見える |
| worktree の一覧の購読 | `usecase/repository_usecase.rs:131-138` | 未コミット数と base ブランチを読めないと、0 と None になる。画面はこの 2 つを使っていない |

## 画面側で購読の失敗を捨てている箇所

- 値だけを取り出す購読（`src/hooks/useStateSubscription.ts:35-39`）を使う 8 か所は、失敗が画面に届かない: `src/components/layout/ProviderHookHealthBanner.tsx:9`、`src/components/workspace/WorkspaceList.tsx:813,917`、`src/components/panels/SettingsModal.tsx:361`、`src/hooks/useCurrentBranch.ts:4`、`src/hooks/useIssues.ts:7`、`src/hooks/useBaseBranch.ts:10,15`
- 購読を直接呼び、失敗を受け取らない 4 か所: `src/hooks/useRepoList.ts:14`、`src/components/workspace/CreateWorktreeModal.tsx:106-117,118-121`、`src/hooks/useWorkspaceTreeNodes.ts:32`
- `src/hooks/useWorkspaceStateCache.ts:47-58` は失敗を受け取るが、「保存された状態が無い」と同じ値にする
- Review の差分の購読（`src/hooks/useReviewSnapshot.ts:21-43`）は、失敗を読み込み中の判定にしか使わない。失敗すると空の snapshot を返し、Review の画面は「No changes」（`src/components/panels/ReviewPanel.tsx:535`）になる
- 起動時のリポジトリ（`src/App.tsx:151-168`）は、読み取りのどの失敗も「git リポジトリの外」として扱い、何も表示しない。リポジトリの追加（`src/App.tsx:176-184`）は、どの失敗も「リポジトリではないフォルダ」として扱い、普通のタブで開く

## 再現の手順

- Automation で workflow を開いたまま、その定義ファイルを壊す。画面には失敗が表示されない（コードで確認。実行はしていない）。
- git の変更の状態の走査が失敗する worktree を Review で開く。画面は「No changes」になる（コードで確認。実行はしていない）。

# Scope / Non-goals

## Scope

- 購読の仕組み: 購読中の読み直しの失敗、外部の情報の取り直しの失敗、端末の表示の取り直しの失敗を画面へ届けること。最初の読み取りに失敗した購読の、その後の読み直し。後から同じ対象を購読した画面への失敗の伝達
- Current Behavior の「読み取りで失敗を『無い・空』に変えている箇所」の全件
- Current Behavior の「画面側で購読の失敗を捨てている箇所」の全件
- worktree の一覧の購読と worktree の作成の応答から、使われていない未コミット数と base ブランチを除くこと
- workflow の定義ファイルの数え上げを、一覧と診断で一つにすること

## Non-goals

- 購読の出力の型のうち、domain の型の扱い。#1955 が扱う
- 監視の失敗（`usecase/state_subscription.rs:278-281,429-431`、`adaptor/gateway/repository/state.rs:100-103,144-150`）。監視が失敗すると後の変化が検出されず、古い値が今の値として出続ける。値の読み取りの失敗とは別の問題として扱う
- `adaptor/presenter/state_subscription.rs:159-161` の、対象の文字列を解析できないときの読み飛ばし。daemon が自分で作った文字列の読み戻しで、読み取りの失敗ではない
- `src/lib/client.ts:355-362` の、知らない種類の値の読み飛ばし。版の違いで知らない種類の値を受け取った場合の扱いで、読み取りの失敗ではない
- `usecase/workspace_tree/list.rs:102` のリポジトリの root の読み取り。同じ失敗が worktree の並びの失敗として値に残る（`usecase/repository_state/service.rs:93-101`）
- `usecase/workspace_tree/list.rs:207` の既定値。集めた値の並びの対応の話で、読み取りの失敗ではない
- `adaptor/gateway/agent_session/agent_session_history_gateway.rs:328` の、UTF-8 でないファイル名の読み飛ばし。provider の session の ID にならない
- `adaptor/gateway/workflow/storage.rs:328-334` の、名前にならないファイル名の読み飛ばし。定義の名前にならないファイルで、読み取りの失敗ではない
- `domain/external_editor/services.rs:22` のエディタの検出。開けない場所のアプリは起動できないので、「無い」とするのが検出の仕様どおり
- `usecase/agent_session/provider_availability.rs:125-135` の、使える provider だけを返すこと。使えない理由は provider の利用可否の購読が運ぶ
- `src/lib/terminalPerformanceSwitches.ts:21-30`、`src/main.tsx:32-34`。この対象の読み取りは固定の値で失敗しない。失敗するのは接続だけ
- `src/hooks/useNotionSettings.ts:90`。失敗を受け取って表示している

# Requirements

- R-001: 購読している状態を daemon が読めなかったとき、画面はそのことを、状態が無い・空であることと区別して表示する。
- R-002: 購読を始めた後に状態を読み直して失敗した場合も、読めなかったことが画面に届く。画面は古い値を今の値として出し続けない。外部の情報の取り直しの失敗と、端末の表示の取り直しの失敗も同じである。
- R-003: 最初の読み取りに失敗した購読も、対象の状態が変われば読み直され、読めれば値が画面に届く。
- R-004: 読めていない対象を後から別の画面が購読したとき、読めないことがその画面にも届く。
- R-005: 読み込めない workflow の定義は、定義が無いことと区別して画面に届く。定義の形式を読めないときも、既定の形式として扱わない。
- R-006: git の変更の状態を読めないとき、Review の画面は差分を読めないことを表示し、「変更が無い」と区別する。一度読めた後に読めなくなった場合も、前の差分を今の差分として出さない。
- R-007: Workspaces の一覧は、未コミット数を読めない worktree について、0 や前の数を正しい値として出さず、読めないことを区別して出す。
- R-008: Workspaces の一覧を集める処理の失敗を、空の一覧として出さない。PR の状態を取れないときも、「PR が無い」と区別する。
- R-009: ワークスペースの保存された状態について、ファイルが無いことと、ファイルを読めない・壊れていることを区別する。読めないときは既定の配置で開いた状態を保存せず、読めなかったファイルを上書きしない。
- R-010: Issue の一覧は、`gh` の失敗（未認証、API のエラーなど）、出力を解析できないこと、origin の URL を読めないことを、「Issue が無い」「GitHub のリポジトリではない」と区別して画面に届ける。
- R-011: branch base と releash base は、未設定であることと、設定を読めないことを区別して画面に届ける。既定ブランチを探すときの失敗も、「既定ブランチが無い」と区別する。
- R-012: git の読み取りで失敗を「無い」として扱うのは、対象が存在しないとき、まだコミットの無いブランチ、実体の無くなった worktree、リポジトリではないパスに限る。後の 3 つは、今までどおり失敗にしない。
- R-013: worktree の名前・lock の状態・ブランチを読めないとき、その worktree を一覧から消したり、lock されていない・ブランチが `"unknown"` として出したりせず、読めないことが画面に届く。
- R-014: session の履歴は、タイトルと最初の入力を読めないとき、タイトルが無いことと区別する。
- R-015: provider hook の警告の記録を読めない・壊れているとき、警告が無いことと区別する。
- R-016: workflow の一覧は、読めない定義ファイルを一覧から消さない。facet を読めないとき、builtin の内容や空の説明を出さない。診断は、定義や facet の置き場所を読めないとき、問題が無いという結果を出さない。一覧と診断は、同じ定義ファイルを対象にする。
- R-017: 次の画面は、購読の失敗を受け取って表示する: provider hook の警告、Workspaces の session 履歴と provider の選択肢、設定画面のブランチの選択肢、今のブランチ、Issue の一覧、base ブランチとその選択肢、リポジトリの一覧、worktree 作成画面のブランチとブランチの状態。archive の後の選択の照合は、選択を読めないとき、そのことを表示する。
- R-018: 起動時のリポジトリの読み取りと、リポジトリの追加は、読み取りの失敗を「git リポジトリの外」「リポジトリではないフォルダ」と区別して表示する。リポジトリの外・リポジトリではないフォルダのときは、今までどおりに動く。
- R-019: worktree の一覧の購読と worktree の作成の応答は、未コミット数と base ブランチを含まない。

# Assumptions

なし
