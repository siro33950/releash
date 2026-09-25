# Design 03

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `9194bd57`（`refactor(state): Workspaces の表示を全て状態購読に移す (#1885) (#1924)`）。この commit に、design-01 と design-02 に基づく未コミットの実装変更が載った状態を今周の開始状態とする。
- 直前の Design は `docs/specs/issues-1890/design-02.md`。design-01 と design-02 の「変える部分」の実装は開始状態に入っている。再掲しない。
- design-02 で扱った 9 件の Thread のうち 8 件（`dc6af34c`・`3da84a05`・`00ef5e4a`・`f76ab557`・`eb542d65`・`fc3b89e1`・`e89119da`・`826ac6b9`）は resolve 済みである。
- `40ec7ee3-660d-4623-a5b2-9faa865fdd71` は design-02 から未達のまま open で残り、`[STILL_OPEN]` が付いている。これに今周の新しい 7 件を加えた `[FIX_POLICY]` 付きの open Thread 8 件が、今周で変える部分である。
- `[REJECTED]`・`[DEFERRED]` の Thread は無い。
- Requirements・Behavior はこの周で変更していない。R-001〜R-011 と B-001〜B-016 の対応に不足・矛盾は無い。

## 変える部分

- 作業列の外にある再試行経路の集約: `usecase/work_queue.rs:535-569` の `retry_with_scope()` が `claim` の後の自前の loop で `ready` / `operation` / `observe` / `failed` を呼び、`enqueue` / `dispatch` の jobs 経路を通らない。`usecase/workflow/node_startup.rs:78-145` の `retry_node()` も `claim`・`ready`・`failed` の独自ループを持つ。これを共通の作業列へ寄せる。根拠: Thread `40ec7ee3-660d-4623-a5b2-9faa865fdd71`、R-001、R-003、design-01 の固定ルート「作業列は 1 つ」。受入条件は「ISSUE 対象の再試行が共通の作業列を通り、同一対象の積み込みの畳み込み・失敗回数・待ち時間の決定を作業列が所有する」。分類から `RetryBackoff::CONFLICT` を選ぶ判断が `WorkQueue::retry` の 1 箇所になったことは開始状態に入っており、残るのは受入条件の前半である。ルート: 委任。

- `spawn_blocking` の `JoinError` の変換の集約: `adaptor/gateway/terminal_surface/checkpoint_scheduler.rs:47-52` と `adaptor/gateway/comment/watcher.rs:58-63` が、`JoinError` を `WorkFailure { kind: FailureKind::Internal, message: error.to_string() }` へ変換する同じ処理を個別に持つ。集約場所 `adaptor/gateway/shared/background_io.rs` は既にあり、両者はその `failure()` を I/O エラーに対してのみ使っている。根拠: Thread `f5c0a864-dc60-4ca5-8879-7914db08f523`、`docs/architecture/README.md:32`「同じ操作の実装は 1 つに集約する。」受入条件は「`spawn_blocking` の `JoinError` から `WorkFailure` への変換が 1 つの実装を通り、terminal の保存と review comment の監視で同じ分類と同じ文面になる」。ルート: 委任。

- `HostNodeStartup::restart` の `RetryAction::Retry` 分岐の gateway 層テストの追加: `adaptor/gateway/workflow/workflow_host/node_startup.rs:35-66` のこの分岐は、`load_control_plane_execution` による実行の再読込、`is_active()` による停止判定、`isolated_composite_start` または `leaf_start_for` による `NodeStart` の再構築を行うが、実物の `HostNodeStartup` で駆動するテストが無い。`workflow_host` 配下のテストに `HostNodeStartup` の出現は無く、usecase 層の `FakeStartup` のテストはこの分岐を通らない。根拠: Thread `e8e73dd2-c864-480e-9095-d54c974fe457`、`docs/architecture/TEST.md:26` の adaptor/gateway 層のテストの必須要件、B-005。受入条件は「`HostNodeStartup::restart` の `RetryAction::Retry` 分岐を実物の gateway で駆動するテストがあり、実行の再読込・停止判定・composite / leaf の再構築のいずれかを誤った場合にそのテストが失敗する」。ルート: 委任。

- 要対応の状態の変化の通知の集約: `usecase/work_queue.rs:370-390` の `observe()` と同 `478-497` の `clear_attention()` が、publisher の取得、`key.operation` が `workflow_` で始まりかつ `attention_changed` のときの `StateChangeSource::WorkspaceList` の invalidation、`StateChangeSource::Failures(key.target)` の invalidation を、同じ条件と同じ順序で個別に実行する。根拠: Thread `2bc9ac4d-3bd3-4f66-820b-4b6a5c98b757`、`docs/architecture/README.md:32`。受入条件は「要対応の状態が変わったときの購読者への通知が 1 つの実装を通り、設定時と解除時で通知対象が構造的に一致する。R-006・B-008・B-014 で観測できる結果は変わらない」。ルート: 委任。

- 失敗の記録の読み取りの QueryService への分離: `usecase/work_queue.rs` の `WorkQueueUsecase` が、状態を変える `enqueue` / `execute` / `dispatch` / `observe` / `clear_attention` と、読み取りの `records`（`:389`）・`records_page`（`:401`）・`observations`（`:429`）・`requires_attention`（`:441`）を同じ struct・同じファイルで提供する。`records_page` は `usecase/state_subscription/reads.rs` から画面用の `FailurePage` の組み立てに使われる。根拠: Thread `2cb66caa-1982-4ab7-93df-ddcb0d2c13a0`、`docs/architecture/USECASE.md:8` の Command と QueryService の分離。受入条件は「失敗の記録の読み取りが QueryService として Command の業務手順とは別ファイルに置かれ、controller からの入口は Usecase に統一される。R-007・B-009・B-015 で観測できる結果は変わらない」。ルート: 委任。

- work queue の依存の composition root での配線: gateway（`adaptor/gateway/terminal_surface/checkpoint_scheduler.rs:33,36`、`adaptor/gateway/comment/watcher.rs:11`、`adaptor/gateway/workflow/workflow_host.rs:1417`、`adaptor/gateway/provider_lifecycle/event_repository_impl.rs:43`、`adaptor/gateway/workspace_tree/query_service.rs:123`）と usecase（`usecase/workflow/startup.rs:27`、`usecase/workflow/node_startup.rs:47`、`usecase/repository_state/worker.rs:68`、`usecase/agent_session/provider_session_title_ingestion.rs:38`、`usecase/state_subscription/reads.rs:59`）が `usecase::work_queue::shared()` をその場で直接呼ぶ。`WorkspaceStateReads` は他の協力者を field で受け取る一方、失敗の記録だけを関数内の `shared()` から取る。根拠: Thread `9b7a6649-1e6e-461b-9dd1-d655fe4f1345`、`docs/architecture/README.md:28`「DI 配線（composition root）は controller の責務とし、gateway や任意のエントリポイントへ配線責務を漏らさない。」受入条件は「work queue への依存が composition root で配線され、gateway と usecase がグローバルの `shared()` をその場で取得しない。依存の差し替えがグローバルの初期化順に依存しない」。ルート: 委任。

- 期限を過ぎた試行が資源を保持し続ける欠陥の解消: `adaptor/gateway/work_queue.rs:57-58` の `attempt` は `tokio::time::timeout(Duration::from_secs(20), attempt)` で future を drop するだけであり、`adaptor/gateway/comment/watcher.rs:41-62` の試行は `spawn_blocking` の中で watcher の `Mutex` を保持したまま `start` と `poll` を実行する。開始済みの `spawn_blocking` は `JoinHandle` の await を中断しても実行が続くため、期限の後もブロッキング処理と lock の保持が残り、同じ watcher を使う後続の試行が lock 待ちになる。根拠: Thread `989ecbe2-65ed-49af-85cc-aad1ba014600`、R-011、B-013。受入条件は「繰り返す処理の 1 回の試行が期限を過ぎたとき、その試行が保持している資源によって、同じ対象の以後の処理と他の対象の処理が止め続けられない」。ルート: 委任。

- `CANCELLED` の観測で残存する要対応が解除されない欠陥の解消: `domain/failure.rs:53-56` の `TargetFailures::observe` は `kind.requires_attention()` が偽のとき既存の entry を消さずに `false` を返す。`usecase/work_queue.rs:441-447` の `requires_attention` は `repository_scan`・`terminal_checkpoint`・`provider_session_title` でこの `TargetFailures` を表示の判定に使うため、`StateRequired` などで要対応になった対象に同じ operation・target の `Cancelled` を観測しても、以前の要対応が残り続ける。`FailureRecords` を使う他の operation では同じ状況で `resolve` される。根拠: Thread `333153df-2f98-4c1b-98ad-d8338d818a35`、R-006、B-014。受入条件は「`repository_scan`・`terminal_checkpoint`・`provider_session_title` の対象が要対応になった後に、同じ対象で `CANCELLED` に相当する分類の失敗を観測したとき、その対象は要対応にならない。R-006 の他の分類での観測結果は変わらない」。ルート: 委任。

## 固定するルート

- この周で新たに固定するルートは無い。8 件すべて実装上のルートは委任する。
- design-01 の「固定するルート」をすべて維持する。維持する対象は、規則の所有（やり直しの判断・待ち時間の計算・やり直しの頻度の上限・失敗の記録・要対応・repository の走査）、待ち時間の値の 4 組、共通の値（ばらつき 0.8〜1.2 倍、全体の上限 10 回/秒・一度に 100、1 回の試行の期限 20 秒）、作業列が client-go の workqueue に従うこと、やり直しの回数に上限を設けないこと、起動時の再開処理を一覧の読み込みも含めて作業列に載せ abort しないこと、「変えないもの」の範囲である。design-02 で維持したこれらを今周も維持する。解除するルートは無い。

## 変えないもの

- design-01 と design-02 の「変えないもの」をすべて維持する。各処理が成功しているときの実行の周期、`usecase/repository_state/worktree.rs` の `WorktreeState` が持つ走査の進行の状態の domain への移設を行わないこと、やり直しの判断と同じ失敗かどうかの判定に失敗の文面・原因の型・エラーの変種を使わないこと、#1879 と #1894 の振る舞いを扱わないこと、R-007 の失敗の記録の保持の件数の上限、R-006・B-008・B-014 で観測できる結果を変えない。
- 1 回の試行の期限の値 20 秒。Thread `989ecbe2` の対象は期限の効かせ方であり、値は design-01 で固定したまま変えない。

## 未確定・リスク

- この周で自動判断した箇所は無い。Requirements・Behavior に誤り・不足・矛盾は無く、いずれのファイルも変更していない。
- この周までに自動判断した箇所は design-01 で記録した 3 件（R-001 の限定、R-010 の R-005 への包含、B-013 の AND の文言）であり、いずれも requirements.md の Assumptions に残っている。
- 未決のまま残した要求は無い。Assumptions に「自動判断: 未決」は無い。
- `[DEFERRED]` で人間へ渡した件は無い。
- Thread `989ecbe2` の対象である `spawn_blocking` の試行は、開始した後に外から停止させる手段が tokio に無い。B-013 の THEN「その試行は打ち切られる」を、開始済みのブロッキング処理そのものの停止として満たす形は未確定であり、想定が外れると B-013 を満たせない。Thread の受入条件が押さえているのは AND の「止め続けられない」側だけである。
