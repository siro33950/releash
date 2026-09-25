# Design 02

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `9194bd57`（`refactor(state): Workspaces の表示を全て状態購読に移す (#1885) (#1924)`）。この commit に、design-01 に基づく未コミットの実装変更が載った状態を今周の開始状態とする。
- 直前の Design は `docs/specs/issues-1890/design-01.md`。design-01 の「変える部分」の実装は開始状態に入っている。再掲しない。
- この周までに解消・見送りとなった Thread は無い。`[REJECTED]`・`[DEFERRED]` の Thread も無い。
- `[FIX_POLICY]` が付いた open Thread が 9 件あり、いずれも今周で変える部分である。
- Requirements・Behavior はこの周で変更していない。R-001〜R-011 と B-001〜B-016 の対応に不足・矛盾は無い。

## 変える部分

- work queue の runtime の配線の是正: `usecase/work_queue.rs:104-115` の `shared()` が `#[cfg(test)]` 分岐で `crate::adaptor::gateway::work_queue::TokioWorkQueueRuntime` を生成する逆依存を解消し、`adaptor/controller/daemon.rs:29` と `adaptor/controller/wiring.rs:466` に分散した `install` を composition root の 1 箇所にする。`install` は `let _ = SHARED.set(..)` で失敗を無視するため後者は効果がない。根拠: Thread `dc6af34c-da7c-4fe3-9829-86b200fe0f0e`。受入条件は「`usecase/work_queue.rs` が `crate::adaptor` を参照しない。work queue の runtime の配線が composition root の 1 箇所で行われ、効果のない `install` の呼び出しが残らない。テスト経路の runtime も usecase 層から adaptor を参照せずに与えられる」。ルート: 委任。

- node 起動失敗の観測と再試行対象化の集約: `workflow_node_start` キーでの `observe` と `FailedNodeStart` の積み込みが `gateway/workflow/workflow_host.rs:1367-1380`、同 `1405-1420`、`gateway/workflow/workflow_host/isolated_worktree.rs:132-149` の 3 箇所に個別実装されており、後続の settle の条件も経路ごとに異なる。これを 1 つの実装に集約する。根拠: Thread `3da84a05-4724-4c44-9082-a083b49ee246`、R-004「同じ分類の失敗は、どの処理で起きても同じ扱いになる」。受入条件は「node 起動失敗の観測と再試行対象化が 1 つの実装を通り、3 経路が同じ分類の失敗に対して同じ観測と同じ再試行対象化を行う」。ルート: 委任。

- 作業列の外にある再試行経路の集約: `usecase/work_queue.rs:515-552` の `retry_with_scope()` が `domain::WorkQueue` の `add` / `get` / `failed` / `done` を通らず自前の loop で再試行し、`usecase/workflow/node_startup.rs:70-118` の `retry_node()` も独自に `retry_delay` と `acquire_retry` を呼ぶ。`RestartRequired` なら `RetryBackoff::CONFLICT` を選ぶ判断が `work_queue.rs:222`・同 `539`・`node_startup.rs:80` の 3 箇所に重複している。これを共通の作業列へ寄せる。根拠: Thread `40ec7ee3-660d-4623-a5b2-9faa865fdd71`、R-001、R-003、design-01 の固定ルート「作業列は 1 つ」「待ち時間の計算は 1 つの実装にする」。受入条件は「ISSUE 対象の再試行が共通の作業列を通り、同一対象の積み込みの畳み込み・失敗回数・待ち時間の決定を作業列が所有する。分類から待ち時間の方針を選ぶ判断が 1 箇所にある」。ルート: 委任。

- 対象ごとの要対応の状態の所有の是正: `domain/repository`・`domain/terminal_surface`・`domain/agent_session` の 3 つの `background_failure.rs` が `pub(crate) struct BackgroundFailureState(pub crate::domain::failure::TargetFailures);` の 1 行で操作を持たず、`usecase/work_queue.rs:90-95` が 3 インスタンスを保持して `observe`（同 `323-341`）・`records`（同 `374-392`）・`clear_attention`（同 `433-458`）の 3 箇所で operation 文字列を match し `.0` を直接呼んでいる。要対応の設定・判定・解除を各ドメインの操作にする。根拠: Thread `00ef5e4a-00cf-47cd-b7dd-499941037c1c`、design-01 の固定ルート「対象ごとの要対応の状態は各ドメインが所有する」。受入条件は「対象ごとの要対応の状態の設定・判定・解除が各ドメインの操作として行われ、usecase に operation 文字列による同じ振り分けが複数箇所に並ばない。R-006・B-008・B-014 の観測結果は変わらない」。ルート: 委任。

- 同一 `WorkKey` の重なりで誤った `Cancelled` が返る欠陥の解消: `usecase/work_queue.rs:154-184` の `enqueue_with_completion()` が `jobs.entry(key).or_insert(..)` を使うため、同一キーの `execute()` が重なると後発の `Entry`（`completed` の oneshot Sender を含む）が drop され、`execute()`（同 `303-306`）が `FailureKind::Cancelled` を返す。`domain/work_queue.rs:30-45` の `add()` は同一キーを dirty にして畳む実装であり、作業列は終了していない。根拠: Thread `f76ab557-c88d-4d69-9c78-17f4c1f5e1a8`、R-004、R-005、B-014。受入条件は「同一 `WorkKey` の呼び出しが重なっても、作業列が動作中であるにもかかわらず `Cancelled` が返ることはない。各呼び出しは実際の成功値または実際の失敗の分類を受け取る」。ルート: 委任。畳み込み後に各呼び出しへ完了を届けるか、重複を明示の契約で拒否するかの選択を含めて委任する。

- 失敗の記録が観測経路で欠落する欠陥の解消: `usecase/work_queue.rs:126` が `FailureRecords::new(4096)` で 4096 件を保持する一方、`records()`（同 `361-370`）が対象で絞り込んだ後に `.take(100)` で無条件に打ち切る。観測経路は `usecase/state_subscription/reads.rs:57-61` の `records(target)` のみで、`proto/client.proto:3202` の `FailureRecords` に続きを取得する項目がない。根拠: Thread `eb542d65-acad-4d80-b6b1-ba28c84ce947`、R-007「この記録は daemon が保持する状態であり、利用者が観測できる」、B-009。受入条件は「daemon が保持している失敗の記録が、件数によって観測経路から欠落しない。保持の上限までの記録について、対象・分類・発生回数・最初と最後の時刻を利用者が観測できる」。ルート: 委任。R-007 の保持の上限は変えない。

- `terminal_checkpoint` と `provider_session_title` の要対応のテストの追加: `usecase/work_queue_test.rs` で要対応の設定と解除まで確認しているのは `repository_scan`（`:37-102`）と `workflow_*`（`:159-199`）だけで、`terminal_checkpoint` は同ファイルに現れず、`provider_session_title` は停止分類で繰り返さないこと（`:104-133`）だけを確認している。根拠: Thread `fc3b89e1-7fb7-40a0-a795-66a9e243531b`、R-006、`docs/architecture/TEST.md` の usecase 層のテストの必須要件。受入条件は「`terminal_checkpoint` と `provider_session_title` について、要対応になる分類の失敗で要対応が観測でき、その後の成功で解除されることを確認するテストがある」。ルート: 委任。

- 全体のやり直しの頻度の上限の結線のテストの追加: 上限を確認しているのは `domain/retry_test.rs` の `RetryBucket` 単体のみで、多数の対象の再試行が `dispatch()` の retrying 分岐から `acquire_retry()` を通ることを確認する経路がない。根拠: Thread `e89119da-7b2e-4cd6-be7e-212a7dca5166`、R-003、B-003、`docs/architecture/TEST.md` の usecase 層のテストの必須要件。受入条件は「多数の対象が同時に失敗する経路で、daemon 全体のやり直しの頻度の上限が適用されることを確認するテストがある。結線が外れた場合にそのテストが失敗する」。ルート: 委任。

- review comment の監視の開始の失敗からの復旧のテストの追加: `adaptor/gateway/comment/watcher_test.rs` の 2 件は正常に開始した後の `events.json` の通知と非対象ファイルの無視だけを検証しており、`adaptor/gateway/comment/watcher.rs:27-37` の `ReviewCommentsWatcher::start` がやり直す分類で失敗してから再度開始される経路を通らない。根拠: Thread `826ac6b9-5464-4d96-9c5e-993c2dc828aa`、R-010、B-012、`docs/architecture/TEST.md` の adaptor/gateway 層のテストの必須要件。受入条件は「監視の開始がやり直す分類で失敗した後、開始がやり直されて監視が動くことを確認するテストがある。開始失敗が以後の動作を止める形へ戻した場合にそのテストが失敗する」。ルート: 委任。

## 固定するルート

- この周で新たに固定するルートは無い。9 件すべて実装上のルートは委任する。
- design-01 の「固定するルート」をすべて維持する。維持する対象は、規則の所有（やり直しの判断・待ち時間の計算・やり直しの頻度の上限・失敗の記録・要対応・repository の走査）、待ち時間の値の 4 組、共通の値（ばらつき 0.8〜1.2 倍、全体の上限 10 回/秒・一度に 100、1 回の試行の期限 20 秒）、作業列が client-go の workqueue に従うこと、やり直しの回数に上限を設けないこと、起動時の再開処理を一覧の読み込みも含めて作業列に載せ abort しないこと、「変えないもの」の範囲である。解除するルートは無い。

## 変えないもの

- design-01 の「変えないもの」をすべて維持する。各処理が成功しているときの実行の周期、`usecase/repository_state/worktree.rs` の `WorktreeState` が持つ走査の進行の状態の domain への移設を行わないこと、やり直しの判断と同じ失敗かどうかの判定に失敗の文面・原因の型・エラーの変種を使わないこと、#1879 と #1894 の振る舞いを扱わないことを変えない。
- R-007 の失敗の記録の保持の件数の上限。Thread `eb542d65` の対象は観測経路であり、保持の上限は変えない。
- R-006・B-008・B-014 で観測できる結果。Thread `00ef5e4a` の対象は要対応の状態の所有者であり、観測できる結果は変えない。

## 未確定・リスク

- この周で自動判断した箇所は無い。Requirements・Behavior に誤り・不足・矛盾は無く、いずれのファイルも変更していない。
- この周までに自動判断した箇所は design-01 で記録した 3 件（R-001 の限定、R-010 の R-005 への包含、B-013 の AND の文言）であり、いずれも requirements.md の Assumptions に残っている。
- 未決のまま残した要求は無い。Assumptions に「自動判断: 未決」は無い。
- `[DEFERRED]` で人間へ渡した件は無い。
- Thread `eb542d65` の受入条件「保持の上限までの記録が件数によって観測経路から欠落しない」は、保持の上限が 4096 件であることと、`AGENTS.md`「full-retention 設計を避ける」「summary、page、id-based operation、delta で足りる場合に…全体を clone / store / recompute / resend しない」が同時に成り立つ形を要する。両立する観測経路の形は未確定であり、想定が外れると R-007 を満たせない。
