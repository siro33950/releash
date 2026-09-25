# Design 04

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `9194bd57`（`refactor(state): Workspaces の表示を全て状態購読に移す (#1885) (#1924)`）。この commit に、design-01・design-02・design-03 に基づく未コミットの実装変更が載った状態を今周の開始状態とする。
- 直前の Design は `docs/specs/issues-1890/design-03.md`。design-01 から design-03 までの「変える部分」の実装は開始状態に入っている。再掲しない。
- design-03 で扱った 8 件の Thread のうち 6 件（`40ec7ee3`・`f5c0a864`・`e8e73dd2`・`2bc9ac4d`・`2cb66caa`・`333153df`）は resolve 済みである。
- `[FIX_POLICY]` 付きの open Thread は 2 件（`989ecbe2-65ed-49af-85cc-aad1ba014600`・`9b7a6649-1e6e-461b-9dd1-d655fe4f1345`）であり、これが今周で変える部分である。
- `[REJECTED]`・`[DEFERRED]` の Thread は無い。
- Requirements・Behavior はこの周で変更していない。R-001〜R-011 と B-001〜B-016 の対応に不足・矛盾は無い。

## 変える部分

- 期限を過ぎた試行が開始済みのブロッキング処理を止めず、占有した資源を返さない欠陥の解消: `adaptor/gateway/work_queue.rs:56-66` の `attempt` は `tokio::time::timeout(Duration::from_secs(20), attempt)` で future の待機を打ち切り `FailureKind::Expired` を返すだけであり、既に開始した `spawn_blocking` は停止しない。対象は `adaptor/gateway/comment/watcher.rs:50-72` の `start` と `poll`、`adaptor/gateway/terminal_surface/checkpoint_scheduler.rs:55-57` の `flush`、`adaptor/gateway/repository/state.rs:270-282` の `scanner.scan` の 3 箇所である。終わらないブロッキング処理は期限の後も走り続け、tokio の blocking スレッドを返さない。根拠: Thread `989ecbe2-65ed-49af-85cc-aad1ba014600`、R-011「繰り返す処理の 1 回の試行には期限がある。終わらない試行が、同じ対象の以後のやり直しや他の対象の処理を止め続けることはない。」、B-013。受入条件は「繰り返す処理の 1 回の試行が期限を過ぎたとき、その試行が行っていた処理は期限の後まで動き続けず、占有していた資源を返す。同じ対象の以後のやり直しと、他の対象の処理が、期限を過ぎた試行によって止め続けられない」。ルート: 委任。

- work queue の依存の composition root での明示的な配線: `usecase/work_queue.rs:109-124` に `SHARED` / `install` / `shared` が残り、`adaptor/controller/daemon.rs:29` の `install` の後に `daemon.rs:54,237,362,417`・`agent_session_wiring.rs:283,333`・`wiring.rs:213,238,414,440,475,593`・`terminal_surface_runtime.rs:65` の composition 関数がグローバルの `shared()` から取得する。gateway と usecase 自身による直接取得は開始状態で解消しており、残るのは依存の差し替えがグローバルの初期化順に依存する側である。根拠: Thread `9b7a6649-1e6e-461b-9dd1-d655fe4f1345`、`docs/architecture/README.md:28`「DI 配線（composition root）は controller の責務とし、gateway や任意のエントリポイントへ配線責務を漏らさない。」受入条件は「work queue への依存が composition root で生成され、各 composition 関数へ明示的に渡される。依存の差し替えがグローバルの初期化順に依存しない。R-001〜R-011 で観測できる結果は変わらない」。ルート: 委任。

## 固定するルート

- この周で新たに固定するルートは無い。2 件とも実装上のルートは委任する。
- design-01 の「固定するルート」をすべて維持する。維持する対象は、規則の所有（やり直しの判断・待ち時間の計算・やり直しの頻度の上限・失敗の記録・要対応・repository の走査）、待ち時間の値の 4 組、共通の値（ばらつき 0.8〜1.2 倍、全体の上限 10 回/秒・一度に 100、1 回の試行の期限 20 秒）、作業列が client-go の workqueue に従うこと、やり直しの回数に上限を設けないこと、起動時の再開処理を一覧の読み込みも含めて作業列に載せ abort しないこと、「変えないもの」の範囲である。design-02・design-03 で維持したこれらを今周も維持する。解除するルートは無い。

## 変えないもの

- design-01・design-02・design-03 の「変えないもの」をすべて維持する。各処理が成功しているときの実行の周期、`usecase/repository_state/worktree.rs` の `WorktreeState` が持つ走査の進行の状態の domain への移設を行わないこと、やり直しの判断と同じ失敗かどうかの判定に失敗の文面・原因の型・エラーの変種を使わないこと、#1879 と #1894 の振る舞いを扱わないこと、R-007 の失敗の記録の保持の件数の上限、R-006・B-008・B-014 で観測できる結果を変えない。
- 1 回の試行の期限の値 20 秒と、期限を過ぎたときに返す `FailureKind::Expired` の分類。Thread `989ecbe2` の対象は期限の効かせ方であり、値と分類は design-01 で固定したまま変えない。

## 未確定・リスク

- この周で自動判断した箇所は無い。Requirements・Behavior に誤り・不足・矛盾は無く、いずれのファイルも変更していない。
- この周までに自動判断した箇所は design-01 で記録した 3 件（R-001 の限定、R-010 の R-005 への包含、B-013 の AND の文言）であり、いずれも requirements.md の Assumptions に残っている。
- 未決のまま残した要求は無い。Assumptions に「自動判断: 未決」は無い。
- `[DEFERRED]` で人間へ渡した件は無い。
- Thread `989ecbe2` の対象である `spawn_blocking` の試行は、開始した後に外から停止させる手段が tokio に無い。B-013 の THEN「その試行は打ち切られる」を、開始済みのブロッキング処理そのものの停止として満たす形は未確定であり、想定が外れると B-013 を満たせない。Thread の受入条件が押さえているのは、期限の後まで動き続けず占有した資源を返すことと、AND の「止め続けられない」側である。
