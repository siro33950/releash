# Design 01

## 開始状態

- 差分の基準は base branch `main`、派生点は branch `feat/issues/1893` の HEAD `9194bd57`。未コミットの変更は `docs/specs/issues-1893/` の文書だけであり、実装の変更は無い。
- 既存の `design-NN.md` は無く、初回の周である。開始状態の実装は `requirements.md` の Current Behavior を参照する。
- この周までに解消・見送りとなった Thread は無い。

## 変える部分

- 期限の表現を domain に作る: 絶対時刻を持ち、期限の合成・期限切れの判定・残り時間の計算を所有する値オブジェクトを新設する。根拠: R-001「呼び出しに付いた期限は、その呼び出しから始まる store の問い合わせ、外部プロセスの実行、lock の待ちへ引き継がれ、いずれもその期限を超えて続かない」、R-004「引き継いだ期限と、その処理の先が持つ期限のうち、先に来る方で終わる」、B-006。ルート: 固定（「固定するルート」の 2）。
- 期限と取り消しを組にした 1 つの値を作る: 「期限切れまたは取り消しで止まるべきか」の判断を domain が所有し、処理の先へはこの値を渡す。取り消しは domain の port として宣言する。根拠: R-002「呼び出しの期限切れと client の中断は、その呼び出しから始まった同期処理の内側まで届く」、R-003、R-005、B-004、B-005、B-009。ルート: 固定（「固定するルート」の 3）。
- 呼び出しの入口で期限の値を読み、取り消しと束ねて処理の先へ運ぶ: 現状は handler が `connectrpc::RequestContext` を `_ctx` として捨て、`ClientApiDeps::execute` の `CancellationToken` も `dispatch_admitted` の先へ渡っていない。根拠: R-001、R-002。ルート: 委任。
- store の読み込みへ引き継ぐ: `ReaderPool::submit` が引き継いだ期限を受け取り、job 取り出し時の 1 回だけの判定を、呼び出し側の待ちと実行中にも効く形にする。根拠: R-001、R-004、B-001、B-006。ルート: 固定（「固定するルート」の 6）と委任。
- SQLite の実行中の文を中断できるようにする: reader の専用スレッドで走っている文を、取り消しの口で止める。根拠: R-002、B-004。ルート: 固定（「固定するルート」の 5）。
- store の書き込みの返事の待ちへ引き継ぐ: `receiver.await` が期限と取り消しで終わるようにする。待ち行列へ admit 済みの job は取り下げない。根拠: R-001、R-007、B-008。ルート: 固定（「固定するルート」の 7）。
- `gh` の実行へ引き継ぐ: `GH_TIMEOUT` の 10 秒と引き継いだ期限の先に来る方で終わり、そのときプロセスを終了させる。根拠: R-001、R-004、R-005、B-002、B-005、B-006。ルート: 固定（「固定するルート」の 5、6）。
- Notion API の実行へ引き継ぐ: `REQUEST_TIMEOUT` の 10 秒と引き継いだ期限の先に来る方で終わる。429 のときの `Retry-After` の待ちも、期限と取り消しで止まれるようにする。再試行の回数と条件は変えない。根拠: R-001、R-002、R-004、B-002、B-006。ルート: 固定（「固定するルート」の 6）と委任。
- `git apply --cached` / `--reverse` の実行へ引き継ぐ: 期限の無い `wait_with_output` を、期限と取り消しで終わり、そのときプロセスを終了させる形にする。根拠: R-001、R-005、B-002、B-005。ルート: 固定（「固定するルート」の 5）。
- git2 の各操作へ引き継ぐ: checkout と fetch は資源の口で操作の途中でも止め、diff / status は口を持たないため操作が終わった時点で止める。根拠: R-002、R-004、B-004、B-009。ルート: 固定（「固定するルート」の 5）と委任（確認点を入れる位置）。
- 期限付きでファイル lock を待つ手続きを 1 か所へまとめる: `comment/mod.rs`、`repository/worktree_operation.rs` の `registry_lock` と `deletion` の 3 箇所を 1 つの手続きにする。`registry_lock` は確認点を持たない `fs2::FileExt::lock_exclusive` のため、`try_lock_exclusive` の繰り返しへ変える。根拠: R-001、R-003、R-005、B-003、B-005。ルート: 固定（「固定するルート」の 4）と委任（再試行間隔）。
- worktree の削除が変更の操作の終わりを待つ処理へ引き継ぐ: `usecase/worktree_operation.rs` の `ready_to_delete` を待つ `notified()` が、期限と取り消しで終わるようにする。状態遷移そのものは `WorktreeOperationState` が引き続き所有する。根拠: R-001、B-003。ルート: 委任。
- `spawn_blocking` の上で始まった同期処理が止まれるようにする: 期限と取り消しを組にした値を同期処理へ渡し、止まれる点を持たない処理には止まれる点を持たせる。根拠: R-002、R-005、B-004、B-005、B-009。ルート: 固定（「固定するルート」の 3、5）。
- 引き継いだ期限の超過と取り消しを失敗の分類へ写す: 期限切れを `FailureKind::Expired`、取り消しを `FailureKind::Cancelled` にする。根拠: R-003、B-001、B-002、B-003、B-004。ルート: 固定（「固定するルート」の 8）。

## 固定するルート

1. 全体の基準は gRPC の deadline propagation に従う。範囲は期限の引き継ぎ、期限の合成、期限切れと取り消しの終わり方の全体。標準が定める形をそのまま採り、標準が定めない点だけ個別に決める。
2. 期限の合成（先に来る方を採る）、期限切れの判定、残り時間の計算は、domain に新設する Deadline 値オブジェクトが所有する。絶対時刻を持ち、`minimum`・`is_expired(now)`・`remaining(now)` を提供する形まで固定する。既存の `domain/git_host/value_objects/cache.rs` の `CacheTtl::is_fresh(fetched_at, now)` が、値オブジェクトが時刻を引数で受け取って判断する同じ形を採っている。
3. 「期限切れまたは取り消しで止まるべきか」の判断は domain が所有し、Deadline と取り消しの port を組にした 1 つの値を処理の先へ渡す。取り消しは domain の port として宣言し、adaptor が `tokio_util::sync::CancellationToken` で実装する。期限と取り消しを 1 つの値で運ぶことと、port による依存関係逆転までを固定する。
4. 期限付きでファイル lock を待つ手続きを 1 か所へまとめる。対象は `comment/mod.rs`、`repository/worktree_operation.rs` の `registry_lock` と `deletion` の 3 箇所。Deadline と取り消しを受けて `try_lock` を繰り返す手続きを 1 つにすることまでを固定する。諦める判断は Deadline が所有し、lock の取得自体は各 gateway が持つ。
5. 中断は資源ごとの口を使う。外部プロセスはプロセスの終了、SQLite の実行中の文は `sqlite3_interrupt`（rusqlite の `InterruptHandle`）、git2 の checkout は `CheckoutBuilder::notify`、git2 の fetch は `RemoteCallbacks::transfer_progress`、ファイル lock は `try_lock` の繰り返し。git2 の diff / status は口を持たないため、その操作が終わった時点で止める。
6. 資源側が持つ期限は削除せず、引き継いだ期限と先に来る方を採る。`ReaderPool` の `QUERY_DEADLINE_MS` の 2 秒を含む。
7. store の書き込みは `queue.admit` を受け付けの確定として扱い、admit 済みの job を取り下げない。
8. 失敗の分類は #1880 の `FailureKind` が引き続き所有し、期限切れを `Expired`、取り消しを `Cancelled` に写す。Connect コードへの対応付けは `adaptor/protocol/connect.rs` の 1 か所のままとする。

## 変えないもの

- 処理の先が現在持つ期限の値。store の読み込みの 2 秒、SQLite の busy_timeout の 2 秒、`gh` の 10 秒、Notion API の 10 秒、review comment のファイル lock の 10 秒。資源側の期限を削除せず引き継いだ期限と先に来る方を採る形にしたため、値そのものを変える根拠がない。
- 呼び出しの期限が引き継がれない経路に、新しい期限を置くこと。対象は daemon の中の繰り返し処理、workflow の runtime、CLI / hook 用の HTTP local API。標準は呼び出しから始まらない処理に既定の期限を置かず、その処理の側が決める。daemon の繰り返し処理の期限は #1890 が担う。
- 呼び出しの期限が引き継がれない経路での lock の待ちの上限。`registry_lock` を `try_lock_exclusive` の繰り返しへ変えるが、引き継いだ期限が無いときは現状どおり取得できるまで待つ。R-006 は開始状態で満たされており、この周で変えない。
- 中断の口を持たない操作に、口を作ること。git2 の diff / status を別プロセスへ切り出すことを含む。ISSUE の実装境界が「止まれない処理は、止まれる点を持つ形にする」であり、1 操作の内側を割ることは境界を超える。
- 失敗の分類の種類。`FailureKind` に新しい種類を追加しない。
- store の入口の形（#1881・#1892 が async にしたもの）と、期限・取り消しを `execute` で受け取る仕組み（#1883）。既定 120 秒と client が指定した期限の扱いを含む。
- reader プールのスレッド数（`READER_POOL_SIZE`）と待ち行列の深さ（`READ_QUEUE_MAX_DEPTH`）。同時実行の枠は #1894 が担う。

## 未確定・リスク

- 自動判断: `requirements.md` の Current Behavior にある git2 の module の列挙を `9194bd57` の実装に合わせて補正した。正本の記載に無い `repository/git_config`、`repository/watch`、`code/diff_compute`、`git_host/discovery`、`workflow/worktree_gateway` が非テストの経路で `git2::` を使う。
- 委任（実装で決める）: 期限の時刻の基準をどれに揃えるか（`StoreClock` / `std::time::Instant` / 単調時計）。各 gateway への期限の渡し方（引数を足すか、既存の deps 構造体へ載せるか）。git2 の diff / status に確認点を入れる位置（操作の前後か、呼び出し元か）。`registry_lock` を `try_lock` のループへ変える際の再試行間隔。期限の定数を持つ箇所の全数の洗い出し。
- `git2::` を使う非テストの call site は `9194bd57` で 120 箇所あり、15 module に散らばる。このうち呼び出しから始まる経路の全数は未確定である。確認点を入れる箇所の洗い出しが漏れると、その経路で R-002 と B-009 を満たさない操作が残る。
