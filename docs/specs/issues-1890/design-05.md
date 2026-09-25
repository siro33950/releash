# Design 05

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `9194bd57`（`refactor(state): Workspaces の表示を全て状態購読に移す (#1885) (#1924)`）。この commit に、design-01 から design-04 に基づく未コミットの実装変更が載った状態を今周の開始状態とする。
- 直前の Design は `docs/specs/issues-1890/design-04.md`。design-01 から design-04 までの「変える部分」の実装は開始状態に入っている。再掲しない。design-04 で扱った 2 件の Thread（`989ecbe2-65ed-49af-85cc-aad1ba014600`・`9b7a6649-1e6e-461b-9dd1-d655fe4f1345`）はいずれも resolve 済みで、期限を過ぎた試行の子プロセス回収は `src-tauri/src/infrastructure/process/attempt.rs` に、work queue の配線は composition root へ移り、`crate::usecase::work_queue::install` は存在しない。
- `[FIX_POLICY]` 付きの open Thread は 2 件（`819bb2d2-6b9a-46ec-b4b3-b08f351411ae`・`04471156-1476-4f41-8a10-aa8a875d77a5`）であり、これが今周で変える部分である。
- `[REJECTED]`・`[DEFERRED]` の Thread は無い。
- Requirements・Behavior はこの周で変更していない。前段が R-012（`docs/specs/issues-1890/requirements.md:112`）と B-017（`docs/specs/issues-1890/behavior.md:109`）を追加し、対応表に `| R-012 | B-017 |` を加えている。R-001〜R-012 と B-001〜B-017 の対応に不足・矛盾は無い。

## 変える部分

- やり直さない分類で失敗した terminal に新しい出力が生じても保存が行われない欠陥の解消: `adaptor/gateway/terminal_surface/checkpoint_scheduler.rs:61-63` が失敗時に `dirty` を true へ戻すため `mark_dirty`（同 40-43）が早期 return し、`usecase/work_queue.rs:311-315` の失敗経路が `state.queue.stop(&key)` を呼ぶため `domain/work_queue.rs:30-33,47-50` が同じ key を `stopped` に保持して以後の `add` を拒否する。`stopped` の解除は `completed.is_some()` の経路（`usecase/work_queue.rs:191-192`）だけで、`enqueue_after` は `completed: None` のため解除されない。根拠: Thread `819bb2d2-6b9a-46ec-b4b3-b08f351411ae`、R-012「やり直さない分類で失敗した対象について、その後に処理すべき新しい変化が生じたときは、その変化に対する処理が行われる。過去の失敗を理由に、その対象の繰り返す処理が daemon を起動し直すまで行われないままになることはない。失敗した試行そのものがやり直されるわけではない。」、B-017。受入条件は「やり直さない分類で terminal checkpoint の保存が失敗した後、その terminal に処理すべき新しい出力が生じたとき、その出力を含む保存が行われる。過去の失敗を理由に、その terminal の保存が daemon を起動し直すまで行われないままにならない。失敗した試行そのものはやり直されない。R-006・B-008・B-014 の要対応の観測は変わらない」。対象は terminal checkpoint の経路である。repository の走査は `usecase/repository_state/worker.rs:87` が `queue.execute` 経由で `stopped` を解除するため対象外であり、時間の経過だけを契機とする処理（review comment の監視、provider session のタイトルの取得）は R-012 の対象ではない。ルート: 委任。

- 今周の実装・テスト・spec 文書が版管理に含まれていない欠陥の解消: `git status --porcelain` の未追跡は 17 件であり、追跡済みのモジュール宣言（`infrastructure/process/mod.rs:1-2`、`adaptor/gateway/shared/mod.rs:2`、`usecase/mod.rs:53,55`、`adaptor/controller/mod.rs:6`）が未追跡の `infrastructure/process/attempt.rs`・`infrastructure/process/background_worker.rs`・`adaptor/gateway/shared/background_worker.rs`・`adaptor/controller/background_worker.rs`・`usecase/failure_query_service.rs`・`usecase/work_queue_test_runtime.rs` とそれらの回帰テストを参照する。版管理された差分だけでは参照先を欠いてビルドできず、R-001〜R-012 のいずれも観測できない。未追跡には今周の spec 文書（`docs/specs/issues-1890/design-02.md`・`design-03.md`・`design-04.md`、およびこの `design-05.md`）も含まれる。根拠: Thread `04471156-1476-4f41-8a10-aa8a875d77a5`、R-011「繰り返す処理の 1 回の試行には期限がある。終わらない試行が、同じ対象の以後のやり直しや他の対象の処理を止め続けることはない。」、B-013、R-007「同じ失敗は 1 件の記録にまとまり…この記録は daemon が保持する状態であり、利用者が観測できる。」受入条件は「追跡済みのモジュール宣言が参照する実装とテストのファイル、および今周までの spec 文書が版管理に含まれ、版管理された差分だけで src-tauri がビルドでき、R-001〜R-012 と B-001〜B-017 が観測できる。R-001〜R-012 で観測できる結果は変わらない」。ルート: 委任。

## 固定するルート

- この周で新たに固定するルートは無い。2 件とも実装上のルートは委任する。
- design-01 の「固定するルート」をすべて維持する。維持する対象は、規則の所有（やり直しの判断・待ち時間の計算・やり直しの頻度の上限・失敗の記録・要対応・repository の走査）、待ち時間の値の 4 組、共通の値（ばらつき 0.8〜1.2 倍、全体の上限 10 回/秒・一度に 100、1 回の試行の期限 20 秒）、作業列が client-go の workqueue に従うこと、やり直しの回数に上限を設けないこと、起動時の再開処理を一覧の読み込みも含めて作業列に載せ abort しないこと、「変えないもの」の範囲である。design-02・design-03・design-04 で維持したこれらを今周も維持する。解除するルートは無い。

## 変えないもの

- design-01・design-02・design-03・design-04 の「変えないもの」をすべて維持する。各処理が成功しているときの実行の周期、`usecase/repository_state/worktree.rs` の `WorktreeState` が持つ走査の進行の状態の domain への移設を行わないこと、やり直しの判断と同じ失敗かどうかの判定に失敗の文面・原因の型・エラーの変種を使わないこと、#1879 と #1894 の振る舞いを扱わないこと、R-007 の失敗の記録の保持の件数の上限、1 回の試行の期限の値 20 秒と期限を過ぎたときに返す `FailureKind::Expired` の分類。
- R-005・B-007 の「失敗した試行そのものはやり直さない」。Thread `819bb2d2` の対象は、失敗の後に生じた新しい変化に対する処理であり、失敗した試行のやり直しではない。
- R-006・B-008・B-014 で観測できる要対応の結果。

## 未確定・リスク

- この周で自動判断した箇所は無い。Requirements・Behavior に誤り・不足・矛盾は無く、いずれのファイルも変更していない。
- この周までに自動判断した箇所は 4 件で、いずれも `docs/specs/issues-1890/requirements.md` の Assumptions に残っている。R-001 の「失敗したとき」を R-005 でやり直す分類に限定したこと、R-010 の開始の失敗を R-005 の判断の対象に含めたこと、B-013 の AND を R-011 の本文どおり「止め続けられることはない」へ揃えたこと、R-012 を追加して「やり直さない分類で失敗した後に新しい変化が生じたときの扱い」を変更前の挙動を維持する側へ寄せたこと。
- 未決のまま残した要求は無い。Assumptions に「自動判断: 未決」は無い。
- `[DEFERRED]` で人間へ渡した件は無い。
