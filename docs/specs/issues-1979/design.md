# Design

## 変える部分
- 購読の worker: `usecase/state_subscription.rs` の `start_read` が起こしている `tokio::spawn`・`loop`・`timer.interval` を usecase から外す。Usecase は 1 回分の操作（対象を 1 回読んで配信する等）だけを公開する。根拠: R-001, R-002, R-004「対象ごとに、変化の知らせ・`notify_and_wait`・外部の情報の取り直しの間隔で読み直して配信する」。ルート: 時刻は infrastructure（`infrastructure/timer.rs` の `ticks`）、task の起動と `loop` は controller。
- `SubscriptionTimer` と `adaptor/gateway/subscription_timer.rs` を削除する。根拠: R-010「`SubscriptionTimer` と `adaptor/gateway/subscription_timer.rs` は存在しない」。ルート: 委任
- terminal の読み直し: `usecase/terminal_surface/subscription.rs` の `schedule_terminal_refresh` が起こしている `tokio::spawn`・`loop` を usecase から外す。Usecase は 1 回分の読み直しと、続けるかの判断を公開する。根拠: R-001, R-005「作り直しの記録が空になったら終わる」。ルート: task の起動と `loop` は controller。
- launch の記録の保持期限: `usecase/agent_session/agent_session_launch.rs` の `tokio::spawn` と `tokio::time::sleep(ACTIVATED_WORKFLOW_LAUNCH_RETENTION)` を usecase から外す。Usecase は記録を 1 件消す操作を公開する。根拠: R-001, R-006「`Activated` になった記録は 300 秒後に消える」。ルート: 遅延は infrastructure の stream、待つことと Usecase の呼び出しは controller。
- terminal の checkpoint の書き出し: `adaptor/controller/terminal_checkpoint.rs` の `tokio::time::sleep(CHECKPOINT_PERSIST_INTERVAL)` を、infrastructure の遅延の stream に置き換える。根拠: R-002, R-007「dirty になった session は 250ms 後に flush される」。ルート: 遅延は infrastructure、task の起動と `loop` は controller に残す。
- repository の scan の worker: `RepositoryStateWorkerRuntime` から `sleep` と `spawn_worker` を外す。debounce の遅延は infrastructure の stream から渡し、worker の起動は controller が持つ。根拠: R-001, R-002, R-008, R-010「`RepositoryStateWorkerRuntime` は `sleep` と `spawn_worker` を持たない」。ルート: 遅延は infrastructure、task の起動と `loop` は controller。
- infrastructure の時刻の駆動部: `infrastructure/timer.rs` の `ticks` に並べて、遅延の stream を置く。根拠: R-002「時刻を作るのは infrastructure の駆動部だけである」。ルート: 委任
- Main の組み立て: 時刻の stream を Main で作って controller に渡すように、`adaptor/controller/daemon.rs`・`adaptor/controller/terminal_surface_runtime.rs` などの組み立てを変える。根拠: R-002, R-009「駆動部が作る時刻のきっかけを、テストが作った偽のきっかけに置き換えて動かせる」。ルート: 委任

## 固定するルート
- 形は、規約どおりの例（provider の session の題名の取り込み）にそろえる。Main が infrastructure の時刻の stream を作って controller の `run` に渡し、controller が stream を待って Usecase を呼ぶ（`adaptor/controller/daemon.rs:226-232`、`adaptor/controller/provider_session_title.rs:9-53`）。
- 時刻を作るのは infrastructure だけ（`infrastructure/timer.rs` の `ticks` と、それに並ぶ遅延の stream）。
- `tokio::spawn` と、きっかけを待つ `loop` は controller が持つ。
- Usecase は 1 回分の操作だけを公開する。JoinHandle・`sleep`・interval を持たない。
- controller に置くのは、きっかけを待つこと、Usecase を呼ぶこと、task の起動と停止だけ。判断は今どおり usecase と domain の関数が答え、controller はそれを呼ぶ。
  - 対象が影響を受けるか（`affected_by`）
  - 外部の情報を取り直すか（`adds_external_information`）
  - 作り直しを続けるか
  - 保持期限が来た記録を消すか
- 購読の変化の知らせの受信は、最初の読み取りより前に始め、その受信を controller の `loop` に引き継ぐ。受信の開始を controller の `loop` の起動まで遅らせない。
- 対象ごとの task の寿命（購読が無くなったら止める）の作りは委任する。ただし、どの対象が今購読されているかの正は usecase の `clients` の 1 つだけにする。controller が対象ごとの JoinHandle を持つなら、止める判断は usecase に問う形にする。

## 変えないもの
- 各処理の間隔と期限: `CacheTtl::EXTERNAL_INFORMATION`、保持期限の 300 秒、checkpoint の 250ms、debounce の 300ms。振る舞いを保つため。
- `RepositoryStateWorkerRuntime` に残る `invalidation_channel`・`scan`・`scan_worktrees` の形。部品の表に無い種類の trait であることはマイルストーン #97 で扱うため。

## 未確定・リスク
なし
