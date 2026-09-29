# Context

- 正本: [#1954 `[10] Output Boundary が一方向になっていない`](https://github.com/siro33950/releash/issues/1954)
- 所属: [milestone #99「02. UI と daemon の間の通信の仕組みを一本化する」](https://github.com/siro33950/releash/milestone/99) の Wave 10。milestone の「層の整理（規約 `docs/architecture/` に合わせる。対象はサーバのコードだけ）」の 7 件のうち 5 番目である。
- 依存: #1931。merge 済みである。#1931 が Output Boundary `StateSubscriptionOutput` と `adaptor/presenter/state_subscription.rs` を作り、画面へ届ける口を presenter へ寄せた。
- 基準は `main` の `9524a6fa`。作業ブランチ `feat/issues/1954` の派生点も同じ commit である。Current Behavior はこの commit で確認した。正本の「今の作り」は `636bb2f0` を基準にしているが、`636bb2f0` と `9524a6fa` の差分に `src-tauri/src/` は含まれないため、正本の記述はそのまま成り立つ。
- 規約の正本は `docs/architecture/`。この変更が根拠にする規定は次のとおり。
  - `README.md`「部品の一覧」: 「domain / Repository の trait / 集約の保存。adaptor/gateway が実装する」。「usecase / Output Boundary の trait / 結果を外へ出す口。画面への状態の配信もここを通る。adaptor/presenter が実装する」。「adaptor/controller / Controller / 外からのきっかけ（転送の要求、時刻・起動、OS の通知）を Input Data に変えて、Usecase を呼ぶ」。「adaptor/presenter / Presenter、転送のメッセージ型 / Output Data を転送の形とステータスコードに変える」。「置いてよい部品は次の表のものだけである。表に無い部品は置かない」。
  - `README.md`「横断的な設計原則」: 「同じ操作の実装は 1 つに集約する。同一の操作が複数箇所に実装されていること自体が問題であり、設定差異・挙動差はその症状にすぎない」。
  - `USECASE.md`「Output Boundary」: 「Usecase が結果を外へ出す口。usecase が trait として定義し、adaptor/presenter が実装する。画面への状態の配信も、この口を通す」。
  - `USECASE.md`「原則」: 「Usecase自身が所有する非同期排他・通知等の実行制御primitiveは使用してよい」。
  - `PRESENTER.md`「原則」: 「変換だけを持つ。業務の判断を書かない。受け取った Output Data を解釈して結果を変えない」。
  - `DOMAIN.md`「trait（Repository、ドメインサービス）」: 「domain 層に置く trait は、ドメインの言語だけで書く」。
  - `DOMAIN.md`「失敗」: 「技術的な失敗が一時的か、やり直してよいかは、失敗を生んだ外側が扱う。domain はそれを判断に使わない」。この規定が禁じているのは、domain が技術的な失敗の性質を判断に使うことであり、値として持つことではない。
- 正本の「今の作り」は `invalidate` の呼び出し元を 21 ファイルとするが、`9524a6fa` で確認したところ 17 ファイルである。残る 4 ファイルは別の `invalidate` を呼んでいる（Current Behavior に記す）。
- 同じ箇所に触れる ISSUE。後から merge する側が、先に merge した変更に合わせる。
  - #1953: `adaptor/controller/api/client_priority.rs`。`FailureOutput` の呼び出し元である。
  - #1952: `adaptor/gateway/desktop_client.rs`・`adaptor/gateway/daemon_supervision.rs`。Tauri のシェルは `FailureOutput` を使わず、監督の Failure に渡す形へ作り替えられる。
- 参照する既存実装: `src-tauri/src/usecase/state_subscription.rs` と `src-tauri/src/usecase/state_subscription/`、`src-tauri/src/usecase/failure.rs`、`src-tauri/src/usecase/retry.rs`、`src-tauri/src/usecase/repo_paths_usecase.rs`、`src-tauri/src/usecase/agent_session/`、`src-tauri/src/usecase/repository_state/worktree.rs`、`src-tauri/src/usecase/comment/`、`src-tauri/src/adaptor/presenter/state_subscription.rs`・`repo_paths.rs`・`agent_session_change.rs`・`repository_state.rs`・`failure.rs`、`src-tauri/src/adaptor/gateway/comment_change.rs`・`failure_records.rs`・`workflow/workflow_host.rs`・`workflow/workflow_host/runtime_session.rs`、`src-tauri/src/adaptor/controller/daemon.rs`・`client/comment/`。

# Outcome

対象者は、daemon を実装・保守する開発者である。Releash の UI と CLI を使う利用者は、この変更の前後で同じ振る舞いを見る。

現在、状態が変わったという知らせは、usecase の Output Boundary の `invalidate` を通って presenter の broadcast に入り、その送り口を組み立てのときに Usecase へ渡すことで Usecase に戻っている。Output Boundary が結果を外へ出す口と、状態の変化が Usecase へ入る経路を兼ねている。知らせを中継する部品は 7 通りあり、そのうち 3 つは usecase から出て usecase に戻るだけである。gateway と presenter が Usecase を通さずに知らせるため、どの状態の変化がどこから来るかを Usecase の入口から追えない。あわせて、`RepositoryPaths` だけが読み直しを通らず直接配信され、失敗の記録は Output Boundary の実装が store へ書いている。

変更後は、Output Boundary は結果を外へ出すだけになる。状態が変わったという知らせは購読の Usecase の入口に集まり、知らせるのは、変化を起こした Usecase と、外からのきっかけを受ける controller になる。presenter は知らせの経路に入らない。`RepositoryPaths` は他の対象と同じ読み直しの経路で配信され、失敗の記録は domain の Repository を通して書かれる。利用者から見た画面の表示、CLI の出力、転送のメッセージと JSON の形、ステータスコードは変わらない。

# Current Behavior

`9524a6fa` のコードで確認した挙動である。

## Output Boundary が入力の経路を兼ねている

- `src-tauri/src/usecase/state_subscription.rs:22-57` の `StateSubscriptionOutput` に `invalidate(source: StateChangeSource)`（`:25`）がある。
- presenter の実装は、知らせを broadcast に送るだけである（`src-tauri/src/adaptor/presenter/state_subscription.rs:316-318`）。presenter は容量 64 の broadcast を持ち（`:43`、`:92`）、その送り口を `change_sender()`（`:61-63`）で公開する。
- 組み立てのときに、`src-tauri/src/adaptor/controller/daemon.rs:52-56` が `state_presenter.change_sender()` を `StateSubscriptionUsecase::new_with_output` へ渡す。
- Usecase は購読を開始するときにその受け口を取り（`usecase/state_subscription.rs:179`）、対象ごとの worker（`:207-244`）が知らせを受けて読み直し、`publisher.publish` で配信する。
- Usecase 自身も、file watch の変化を自分に戻すために `publisher.invalidate` を呼ぶ（`usecase/state_subscription.rs:289`）。

## `invalidate` の呼び出し元は 17 ファイルである

- usecase 10 ファイル: `state_subscription.rs:289`、`workspace_state/usecase.rs:14`、`repository_usecase.rs:85`、`agent_session/provider_availability.rs:120`、`git_host/git_host_usecase.rs:57`、`notion/usecase.rs:38`、`app_config/usecase.rs:49`、`external_editor/open_usecase.rs:36`、`workflow/execution_archive.rs:123,174`、`provider_lifecycle/hook_health.rs:108`。
- gateway 2 ファイル: `comment_change.rs:12`、`workflow/workflow_host/runtime_session.rs:7`。
- presenter 4 ファイル: `repository_state.rs:17`、`agent_session_change.rs:17`、`repo_paths.rs:18`、`failure.rs:34,36`。
- controller 1 ファイル: `daemon.rs:325`。
- 正本が挙げる残り 4 ファイルの `invalidate` は、`WorktreeState::invalidate(InvalidateReason)`（repository snapshot worker への再計算要求、`usecase/repository_state/worktree.rs:214`）であり、`StateSubscriptionOutput::invalidate` ではない。内訳は `usecase/repository_state/service.rs:674,856`、`usecase/repository_state/worktree.rs:210`、`adaptor/gateway/repository/scanner.rs:192`、`adaptor/gateway/repository/state.rs:204,211` であり、`service.rs` の 2 件と `scanner.rs` の 1 件は `#[cfg(test)]` の中にある。

## 状態が変わったことを知らせる形が 7 通りある

1. usecase 10 ファイルが `StateSubscriptionOutputRef` を直接持ち、`invalidate` を呼ぶ。
2. `RepoPathsNotifier`（`usecase/repo_paths_usecase.rs:14`）を `RepoPathsUsecase`（`:44,55`）が呼び、presenter の `RepoPathsNotifyGateway`（`adaptor/presenter/repo_paths.rs:16-26`）が `invalidate(Repositories)` と `publish(RepositoryPaths, paths)` を行う。
3. `AgentSessionChangeNotifier`（`usecase/agent_session/agent_session_change_notifier.rs:3`）を usecase 4 ファイルの計 9 か所（`agent_session_lifecycle.rs:318,337,371,473,638,698`、`provider_lifecycle/ingress.rs:286`、`agent_session_rename.rs:79`、`provider_session_title_ingestion.rs:133`）が呼び、presenter の `ClientAgentSessionChangeNotifier`（`adaptor/presenter/agent_session_change.rs:16-19`）が `invalidate(Worktree(path))` だけを行う。
4. `RepositoryStateNotifier`（`usecase/repository_state/worktree.rs:32`）を snapshot worker（`:255-256`）が呼び、presenter の `ClientRepositoryStateNotifier`（`adaptor/presenter/repository_state.rs:15-18`）が `invalidate(Repository(paths))` だけを行う。
5. `CommentChangeGateway`（`adaptor/gateway/comment_change.rs:10-13`）を controller（`adaptor/controller/client/comment/commands.rs:42,69,98,117`）が呼び、`invalidate(ReviewComments(..))` を行う。
6. `broadcast_state`（`adaptor/gateway/workflow/workflow_host/runtime_session.rs:6-9`）を gateway の 6 か所（`workflow_host.rs:785,998,1300,2046`、`workflow_host/node_startup.rs:259,340`）が呼び、`invalidate(Worktree(path))` を行う。
7. `FailurePresenter::publish`（`adaptor/presenter/failure.rs:29-37`）と、composition root の closure（`adaptor/controller/daemon.rs:323-326`、`WorkspaceListUsecase::with_notifier`、`usecase/workspace_tree/list.rs:122`）。

2・3・4 は、usecase から出て presenter を経て usecase へ戻るだけであり、presenter は変換を持たない。

## `RepositoryPaths` だけ配信の経路が違う

- `usecase/state_subscription.rs:167-169` が `RepositoryPaths` を特別扱いし、読み取りも worker の起動も行わずに購読者の登録だけを行う。
- presenter が組み立てのときに `RepositoryPaths` を登録し（`adaptor/presenter/state_subscription.rs:72-88`）、その保持を固定する（`:89`）。
- `adaptor/controller/daemon.rs:238-241` が初期値を直接 publish する。
- `RepoPathsNotifyGateway::notify_changed`（`adaptor/presenter/repo_paths.rs:19-25`）が、変化のたびに直接 publish する。
- 一方で `usecase/state_subscription/reads.rs:257` には `RepositoryPaths` の読み取りがあり、読み直しでも配信できる。`usecase/state_subscription/target.rs:135` の `StateChangeSource::Repositories` は `SubscriptionTarget::Workspaces` にしか対応していない。

## 失敗の記録を Output Boundary の実装が書いている

- `usecase/failure.rs:128-133` の `FailureOutput` は Output Boundary であり、`adaptor/presenter/failure.rs:40-55` が実装する。
- その実装は、gateway の `FailureRecordStore`（`adaptor/gateway/failure_records.rs:11`）へ `observe` / `resolve`（`:24,70`）で記録し、`invalidate(Failures(target))` を呼ぶ。`workflow_` で始まる operation で注目状態が変わったときは `invalidate(WorkspaceList)` も呼ぶ（`adaptor/presenter/failure.rs:29-37`）。
- 購読の `Failures` は、同じ store を `FailureQueryService`（`usecase/failure.rs:136`、実装は `adaptor/gateway/failure_records.rs:125`）を通して読み直す（`usecase/state_subscription/reads.rs:182-188`）。
- `FailureOutput` の呼び出し元は、usecase（`usecase/retry.rs:33,40,64`）、controller（`adaptor/controller/api/client_priority.rs:56`）、gateway（`adaptor/gateway/desktop_client.rs:184,223`、`adaptor/gateway/daemon_supervision.rs`、`adaptor/gateway/workflow/workflow_host.rs:1399`）である。
- 失敗の性質による判断（`requires_attention`、`next_attempt`、`usecase/failure.rs:140,148`）は usecase にある。gateway の `FailureRecordStore` は `requires_attention` を 6 か所で呼ぶ。書き込み側が `adaptor/gateway/failure_records.rs:33,37,74`、読み取り側が `:86,110,136` である。
- `usecase/retry.rs:76-100` のテスト用の読み取りは、`FailureOutput::as_any` で `FailurePresenter` へ downcast して store を取り出している。

# Scope / Non-goals

今回変更する対象。

- 状態が変わったという知らせを受ける入口を購読の Usecase に置き、知らせを保持して配る仕組み（broadcast）を購読の Usecase が持つこと。
- `StateSubscriptionOutput` から `invalidate` を無くすこと。presenter から broadcast と `change_sender` を無くし、`daemon.rs` が送り口を Usecase へ渡すのをやめること。
- 17 ファイルの `invalidate` の呼び出しを、購読の Usecase の入口の呼び出しに置き換えること。
- 中継の削除。`RepoPathsNotifier`・`AgentSessionChangeNotifier`・`RepositoryStateNotifier` の 3 つの trait と、`adaptor/presenter/repo_paths.rs`・`agent_session_change.rs`・`repository_state.rs`、`adaptor/gateway/comment_change.rs`、`WorkspaceListUsecase::with_notifier`（`usecase/workspace_tree/list.rs:122`）と composition root の closure（`adaptor/controller/daemon.rs:323-326`）を無くし、呼び出し元が購読の Usecase の入口を直接呼ぶこと。
- review comment の変化を、controller ではなく `ReviewCommentUsecase` から知らせること。
- `RepositoryPaths` を、他の購読の対象と同じ読み直しの経路にすること。直接の publish、購読開始時の特別扱い、presenter での事前登録の廃止と、変化の発生源と対象の対応の追加を含む。
- `FailureOutput` と `adaptor/presenter/failure.rs` を無くし、失敗を受け取る Usecase を置くこと。失敗の記録の型（`FailureKey`・`FailureRecord`・`Failure`・`BusinessFailure`・`WorkFailure`）と、記録を書き込む Repository の trait を domain へ移し、`FailureRecordStore` がその Repository を実装すること。要対応かどうかの判断は失敗を受け取る Usecase が行い、その値を Repository へ渡すこと。gateway が `requires_attention` を呼ぶ 6 か所を、渡された値を使う形にすること。状態が変わったという知らせは、その Usecase が行う。
- workflow host が持つ知らせの宛先（`adaptor/gateway/workflow/workflow_host.rs:89`）を、購読の Usecase の入口にすること。
- 上記に伴い使われなくなるコードの削除。

今回変更しない対象。

- `WorktreeState::invalidate(InvalidateReason)` の経路。Usecase の中の snapshot worker への再計算要求であり、Output Boundary を通らない。
- workflow の実行の手順が gateway にあること。`broadcast_state` を呼ぶ 6 か所が gateway から状態の変化を知らせているのはこれが原因であり、原因が別の問題なのでこの変更では扱わない。この 6 か所は R-004 の形に当てはまらないまま残る。
- 失敗の性質による判断（`requires_attention`・`next_attempt`）の置き場所。usecase に残す。
- 失敗の読み取り（`FailureObservation`・`FailurePage`・`FailureQueryService`）。usecase の Output Data と QueryService のままにする。
- 配信する状態の型と、転送の形への変換。#1955 が扱う。
- 読み取りの失敗の出し方。#1956 が扱う。
- `docs/architecture/` の記述。
- 画面（`src/`）と CLI の振る舞い。
- 転送のメッセージ、JSON の形、Connect のステータスコード、HTTP status。

# Requirements

- R-001: usecase の Output Boundary は、結果を外へ出す口だけである。状態が変わったという知らせを受け取る口を持たない。
- R-002: 状態が変わったという知らせは、購読の Usecase の入口に届く。知らせを保持して購読の対象へ配る仕組みは、購読の Usecase が持つ。
- R-003: presenter は、状態が変わったという知らせの経路に入らない。presenter の実装を別の転送のものに差し替えても、状態の変化は購読の Usecase に届く。
- R-004: 状態が変わったことを知らせるのは、変化を起こした Usecase と、外からのきっかけを受ける controller である。知らせを受け取って渡すだけの部品は無い。
- R-005: 同じ状態を配信する経路は 1 つである。`RepositoryPaths` も、他の購読の対象と同じく、購読の Usecase が読み直して配信する。
- R-006: 失敗の記録は、Output Boundary ではなく domain の Repository を通して書き込む。要対応かどうかの判断は、失敗を受け取る Usecase が行う。その Usecase が、記録と、状態が変わったという知らせを行う。
- R-007: 画面と CLI が受け取る転送のメッセージ、JSON の形、Connect のステータスコード、HTTP status は、この変更の前と同じである。
- R-008: 状態が変わったとき、購読している画面が受け取る内容と、受け取る時点は、この変更の前と同じである。
- R-009: この変更で使われなくなるコードは残らない。

# Assumptions

人間が明示的に受け入れた仮定は無い。
