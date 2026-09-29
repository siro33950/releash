# Design 01

## 開始状態

- 差分の基準は `main` の `9524a6fa`。作業ブランチ `feat/issues/1954` の派生点も同じ commit である。
- `docs/specs/issues-1954/` に既存の Design は無い。初回であり、開始状態は `requirements.md` の Current Behavior が示す。
- この周までに閉じた Thread は無い。

## 変える部分

- 知らせの入口: 状態が変わったという知らせを受ける口を購読の Usecase に置き、知らせを保持して配る仕組みを購読の Usecase が持つ。根拠: R-002「状態が変わったという知らせは、購読の Usecase の入口に届く。知らせを保持して購読の対象へ配る仕組みは、購読の Usecase が持つ」。ルート: 委任
- Output Boundary: `StateSubscriptionOutput` から `invalidate` を外す。presenter から broadcast と `change_sender` を外し、`daemon.rs` が送り口を Usecase へ渡すのをやめる。根拠: R-001「usecase の Output Boundary は、結果を外へ出す口だけである」、R-003「presenter は、状態が変わったという知らせの経路に入らない」。ルート: 委任
- 呼び出しの置き換え: 17 ファイルの `invalidate` の呼び出しを、購読の Usecase の入口の呼び出しに置き換える。根拠: R-002、R-004「状態が変わったことを知らせるのは、変化を起こした Usecase と、外からのきっかけを受ける controller である」。ルート: 委任
- 中継の削除: `RepoPathsNotifier`・`AgentSessionChangeNotifier`・`RepositoryStateNotifier` の 3 つの trait と、presenter の 3 ファイル、`adaptor/gateway/comment_change.rs` を無くす。根拠: R-004「知らせを受け取って渡すだけの部品は無い」。ルート: 呼び出し元が購読の Usecase の入口を直接呼ぶ（「固定するルート」）
- Workspaces の一覧: `WorkspaceListUsecase::with_notifier`（`usecase/workspace_tree/list.rs:122`）と composition root の closure（`adaptor/controller/daemon.rs:323-326`）を無くす。根拠: R-004「知らせを受け取って渡すだけの部品は無い」。ルート: 固定（「固定するルート」）
- review comment: 知らせを controller から `ReviewCommentUsecase` へ移す。根拠: R-004。ルート: 固定（「固定するルート」）
- `RepositoryPaths`: 直接の publish をやめ、読み直しの対象にする。購読開始時の特別扱い（`usecase/state_subscription.rs:167-169`）、presenter の事前登録（`adaptor/presenter/state_subscription.rs:72-89`）、`daemon.rs:238-241` の初期配信を無くし、`StateChangeSource::Repositories` と `SubscriptionTarget::RepositoryPaths` の対応（`usecase/state_subscription/target.rs:135`）を足す。根拠: R-005「同じ状態を配信する経路は 1 つである」。ルート: 固定（「固定するルート」）
- 失敗: `FailureOutput` と `adaptor/presenter/failure.rs` を無くし、失敗を受け取る Usecase を置く。記録の型と書き込みの Repository の trait を domain へ移し、判断は usecase に残す。根拠: R-006「失敗の記録は、Output Boundary ではなく domain の Repository を通して書き込む。要対応かどうかの判断は、失敗を受け取る Usecase が行う」。ルート: 固定（「固定するルート」）
- workflow host: 知らせの宛先（`adaptor/gateway/workflow/workflow_host.rs:89` の `state_changes`）を購読の Usecase の入口にする。根拠: R-001「usecase の Output Boundary は、結果を外へ出す口だけである」、R-002。ルート: 固定（「固定するルート」）
- 使われなくなるコードの削除。根拠: R-009「この変更で使われなくなるコードは残らない」。ルート: 委任

## 固定するルート

- 中継の削除: `RepoPathsNotifier`・`AgentSessionChangeNotifier`・`RepositoryStateNotifier` の 3 つの trait と、`adaptor/presenter/repo_paths.rs`・`agent_session_change.rs`・`repository_state.rs`、`adaptor/gateway/comment_change.rs` を残さない。呼び出し元（`RepoPathsUsecase`、agent session の 4 つの usecase、`WorktreeState` の snapshot worker、`ReviewCommentUsecase`）が購読の Usecase の入口を直接呼ぶ。
- Workspaces の一覧: `WorkspaceListUsecase` が購読の Usecase の入口を直接呼ぶ。`with_notifier` と、composition root が渡す closure を残さない。
- review comment: `adaptor/controller/client/comment/commands.rs:42,69,98,117` で知らせるのをやめ、`ReviewCommentUsecase` の `create_thread` / `append_comment` / `resolve_thread` / `delete_thread` から知らせる。controller に「どの状態が変わったか」の判断を残さない。
- `RepositoryPaths`: `RepoPathsUsecase` は変化を購読の Usecase の入口へ知らせるだけにし、`publish(RepositoryPaths, paths)` を行わない。同じ状態を出す経路を 2 つにしない。
- 失敗: 失敗の記録は外へ出す結果ではなく保存する状態なので、Output Boundary ではなく domain の Repository への書き込みとして扱う。置き場所は次のとおり。
  - domain: 記録の型（`FailureKey`・`FailureRecord`・`Failure`・`BusinessFailure`・`WorkFailure`）と、書き込みの Repository の trait。技術的な失敗の性質（`TechnicalFailureNature`）は既に domain にある。
  - usecase: 判断（`requires_attention`・`next_attempt`）。失敗を受け取る Usecase が要対応かどうかを決め、その値を Repository へ渡す。その Usecase が、記録と、状態が変わったという知らせを行う。
  - gateway: `FailureRecordStore` が Repository を実装する。`adaptor/gateway/failure_records.rs` が `requires_attention` を呼ぶ 6 か所（`:33,37,74,86,110,136`）は、渡された値を使う形にする。
  - 読み取り（`FailureObservation`・`FailurePage`・`FailureQueryService`）は、usecase の Output Data と QueryService のままにする。
- workflow host: `broadcast_state` を呼ぶ 6 か所は残し、`state_changes` の宛先だけを購読の Usecase の入口にする。

## 変えないもの

- `WorktreeState::invalidate(InvalidateReason)` の経路。Usecase の中の snapshot worker への再計算要求であり、Output Boundary を通らないため。
- workflow の実行の手順が gateway にあること。`broadcast_state` を呼ぶ 6 か所が gateway から状態の変化を知らせているのはこれが原因であり、原因が別の問題であるため。この 6 か所は R-004 の形に当てはまらないまま残る。
- 失敗の性質による判断（`requires_attention`・`next_attempt`）の置き場所。usecase に残す。`DOMAIN.md`「失敗」が「技術的な失敗が一時的か、やり直してよいかは、失敗を生んだ外側が扱う。domain はそれを判断に使わない」と定めており、判断は domain へ移せないため。同じ規定は、domain が失敗の性質を値として持つことは禁じていない。
- 失敗の読み取り（`FailureObservation`・`FailurePage`・`FailureQueryService`）。usecase の Output Data と QueryService のままにする。

## 未確定・リスク

- `FailureOutput` の呼び出し元のうち、`adaptor/controller/api/client_priority.rs` は #1953 が、`adaptor/gateway/desktop_client.rs`・`adaptor/gateway/daemon_supervision.rs` は #1952 が、同じ箇所を作り替えている。後から merge する側が、先に merge した変更に合わせる。
