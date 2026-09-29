# Context

- 正本: [#1952 `[10] Tauri のシェルの生存確認と設定の受け取りが、#1879 の意図からずれている`](https://github.com/siro33950/releash/issues/1952)
- 調査基準は `main` の `9524a6fa`。作業ブランチ `feat/issues/1952` の派生点は `636bb2f0` で、#1951 を含まない。
- アーキテクチャの正本は `AGENTS.md`。アプリケーションロジックは daemon（サーバ）が所有し、Tauri のシェルは Rust で書かれていても client である。
- #1879（CLOSED、`6146ff51`）が、シェルの生存の判定を購読の stream に流れる bookmark へ置き換えた。1 回の失敗で「居ない」と判定しないこと、判定を 1 つの実装に持たせること、待ちを common の計算で行うことを方針として含む。確認の間隔・打ち切り時間・連続失敗の回数は #1879 では未決のまま、実装の値で決まった。
- #1951（CLOSED、`9524a6fa`）が、client の規則を proto の service option の 1 か所に置いた。`proto/client.proto:2696-2708` に `connection_backoff`（初回 1000ms・1.6 倍・jitter 0.2・上限 120000ms・`reset_after_ms` 120000ms）、`reconnect_status_code`（`RESOURCE_EXHAUSTED` / `ABORTED` / `UNAVAILABLE`）、`state_stream_silence_ms`（30000）、`default_timeout_ms`（120000）がある。option の定義は `proto/client_options.proto:25-44`。
- 画面（React）はこの 4 つを `getOption` で読む（`src/lib/client.ts:51,165,168-170`、`src/lib/connectionBackoff.ts:5`）。Rust 側は proto の option を読んでいない。
- proto の descriptor は `client_descriptor.bin` として実行時に `DescriptorPool` へ読み込まれており（`src-tauri/src/adaptor/presenter/client/json.rs:5-12`）、`prost-reflect`（`src-tauri/Cargo.toml:83`）が依存に入っている。
- 生存確認の作りは gRPC Health Checking Protocol（https://github.com/grpc/grpc/blob/master/doc/health-checking.md ）と Kubernetes の probe（https://kubernetes.io/docs/tasks/configure-pod-container/configure-liveness-readiness-startup-probes/ ）に従う。連続した失敗の回数で判定し、1 回の失敗では判定しない。
- 切断と判定した後の状態遷移と復元の手続きは #1896 と #1904、画面側の接続の判断は #1895 が扱う。

# Outcome

対象者は、Releash のデスクトップ版を使う利用者と、UI と daemon の間の通信を保守する開発者である。

現在、daemon が応答しないまま止まっても、シェルがそれに気づくまで約 62 秒かかる。#1879 の前は約 5〜10 秒だった。その間、画面は daemon が生きているものとして扱われる。シェルが観測した接続の失敗は、どこからも読めない記録先に書かれ、監督へ渡るときに分類が落ちるため、何が起きたのかを後から追えない。シェルは設定を受け取るために 2 本目の stream を開いており、そこだけが分類も上限も無い 1 秒固定のループでつなぎ直している。daemon の側にも、購読が snapshot の作成を待っている間は stream に何も流さない状態があり、生きている daemon との接続を切れたと判定しうる。

変更後は、daemon が開いている全ての stream に一定の間隔で合図が流れ、合図が途絶えたことだけが生存の失敗になる。シェルは daemon との stream を 1 本だけ開き、そこで生存の合図と desktop 設定の両方を受け取る。応答しない daemon に気づくまでの時間は約 41 秒に縮む。つなぎ直しの待ち、つなぎ直しの対象にする失敗、単発の呼び出しの既定の期限、無音と判断する時間は、画面とシェルが proto の同じ規則を読む。シェルが観測した接続の失敗は、分類を保ったまま画面から観測できる。

# Current Behavior

調査基準 `9524a6fa` のコードで確認した挙動である。

## シェルの生存の判定は約 62 秒かかる

- シェルは `observe`（`src-tauri/src/adaptor/gateway/desktop_client.rs:194-236`）で購読の stream を開き、`ATTEMPT_LIMIT`（20 秒、`src-tauri/src/usecase/failure.rs:58`）の間に何も届かなければ stream を閉じて失敗を返す。
- `watch`（`:175-192`）が `DaemonLiveness`（`src-tauri/src/domain/daemon_supervision.rs:9-24`）で連続の失敗を数え、`LIVENESS_FAILURE_THRESHOLD`（3、`:6`）に達すると task を終える。`connected()` は task が終わったかどうかで表される（`desktop_client.rs:139-141`）。
- 失敗の間の待ちは `RetryBackoff::RECOVERY`（初回 800ms・2 倍・上限 30 秒、`src-tauri/src/common/retry.rs:13`）で、jitter は ±20%（`:122-125`）。
- daemon が応答しないまま止まった場合、20 + 0.8 + 20 + 1.6 + 20 ≒ 62 秒で「居ない」と判定する。daemon のプロセスが消えている場合は `open_state_stream` がすぐ失敗するため数秒で判定する。
- `ATTEMPT_LIMIT` は daemon の中の繰り返し処理と共用の定数である（`src-tauri/src/adaptor/controller/workflow_startup.rs:23,33`、`src-tauri/src/adaptor/controller/provider_session_title.rs:20,40`、`src-tauri/src/usecase/workflow/node_startup.rs:109`、`src-tauri/src/adaptor/gateway/shared/background_worker.rs:92`）。
- `watch` は失敗の分類を見ない。`liveness_failure`（`desktop_client.rs:238-250`）が Connect のエラーコードを `TechnicalFailureNature` へ写すが、判定はコードを問わず一律に数える。

## daemon が合図を送らない状態がある

- 合図の間隔は 10 秒（`BOOKMARK_INTERVAL`、`src-tauri/src/infrastructure/state_subscription.rs:14`）。
- 購読を 1 つも持たない stream では、`bookmark()` が false を返し（`:812-826` の `!client.subscriptions.is_empty()`）、stream 全体の合図が流れる（`:162-172`）。シェルの生存確認用の stream は購読を持たないため、この経路で 10 秒ごとに合図を受け取っている。
- 購読を持つ stream では、`bookmark()` が購読ごとに合図を積むが、snapshot の作成を待っている購読（`overflowed`）には積まない（`:817`）。それでも `bookmark()` は true を返すため、stream 全体の合図も流れない。全ての購読が snapshot を待っている間、stream には何も流れない。
- 画面はこの経路の stream を使う。無音が `state_stream_silence_ms`（30 秒、`proto/client.proto:2707`）を超えると stream を切ってつなぎ直す（`src/lib/client.ts:165,245-249`）。
- シェルの設定用の stream も購読を持つため、同じ状態になる。

## シェルは stream を 2 本開き、片方だけ独自のループでつなぎ直す

- `DesktopClient::start`（`desktop_client.rs:105-138`）が 2 つの task を作る。設定用の task は `receive_desktop_settings`（`:63-103`）で 2 本目の stream を開き、`desktop-settings` を購読する。生存確認用の task は `watch` でもう 1 本の stream を開く。こちらは購読を始めない。
- 設定用の task は、stream が終わるたびに分類も上限も見ずに 1 秒だけ待って開き直す（`:116-125`）。
- 接続の確立は、設定の最初の値を 500ms 待って成立する（`src-tauri/src/adaptor/gateway/daemon_supervision.rs:98-112`）。

## シェルの失敗の記録はどこからも読まれない

- `desktop.rs:78-81` が `FailureRecordStore::default()` をその場で作り、`FailurePresenter` へ渡している。この store を保持する他の場所は無く、読み出し口（`src-tauri/src/adaptor/presenter/failure.rs:19-27`）は `#[cfg(test)]` である。`output` も `None` なので配信もされない。
- daemon 側は同じ型を読み出せる形で組み立てている（`src-tauri/src/adaptor/controller/daemon.rs:30,59`）。読まれないのはシェル側の instance だけである。
- シェルが数えた失敗は `Failure { stage: FailureStage::Initialization, reason: <文字列> }` として監督へ渡り、分類は落ちる（`src-tauri/src/adaptor/gateway/daemon_supervision.rs:298-309`）。
- 画面が読めるのは `DaemonStatus`（`src-tauri/src/usecase/daemon_supervision.rs:52-60`）の `stage` と `reason` である（`get_daemon_status`、`src-tauri/src/adaptor/controller/command/desktop_lifecycle.rs:38-40`）。

## シェルの単発の呼び出しの既定の期限は Rust の中にある

- シェルの単発の呼び出し用の client は既定の期限 30 秒を持つ（`desktop_client.rs:26-33`）。stream 用の client は既定の期限を持たない（`:35-42`）。
- 画面の既定の期限は proto の `default_timeout_ms`（120000）を読む（`src/lib/client.ts:51`）。

# Scope / Non-goals

今回変更する対象。

- daemon が合図を流す条件（`src-tauri/src/infrastructure/state_subscription.rs`）
- シェルの生存の判定（`src-tauri/src/adaptor/gateway/desktop_client.rs`、`src-tauri/src/domain/daemon_supervision.rs` の `DaemonLiveness`）
- シェルの stream の本数と設定の受け取り（`src-tauri/src/adaptor/gateway/desktop_client.rs`）
- シェルのつなぎ直しの待ちとつなぎ直しの対象の決め方、単発の呼び出しの既定の期限の出どころ
- proto の `state_stream_silence_ms` の値（`proto/client.proto`）
- シェルの失敗の記録先と、監督への失敗の渡し方（`src-tauri/src/desktop.rs`、`src-tauri/src/adaptor/gateway/daemon_supervision.rs`）

今回変更しない対象。

- 合図の間隔（10 秒）。届く頻度は変えない
- 切断と判定した後の状態遷移と、復元の手続き。#1896 と #1904 が扱う
- 画面（`src/lib/client.ts`）の接続の判断のコード。#1895 が扱う。ただし `state_stream_silence_ms` の値を変えるため、画面が stream を切ってつなぎ直す契機は 30 秒から 20 秒になる
- `RetryBackoff::SERVICE`（`src-tauri/src/common/retry.rs:15`）と proto の `connection_backoff` が同じ値を二重に持つこと。別途報告する
- daemon の中の繰り返し処理が使う `ATTEMPT_LIMIT` の値
- 購読の版からの再開の仕組みと、購読ごとの版を運ぶ合図の意味
- CLI / hook 用の HTTP local API

# Requirements

- R-001: daemon は、開いている購読の stream ごとに、購読の有無と snapshot の作成を待っているかどうかにかかわらず、一定の間隔で client へ合図を流す。
- R-002: シェルは、daemon からの合図が proto の規則で定めた無音の時間を超えて途絶えたときと、stream が切れたときに、生存の失敗として 1 回数える。購読の開始と停止の失敗は生存の失敗として数えない。
- R-003: シェルは、生存の失敗が連続して 2 回に達したときにだけ、daemon が「居ない」と判定する。合図が届いた時点で連続の数えは 0 に戻る。
- R-004: シェルは daemon との購読の stream を 1 本だけ開き、その stream で生存の合図と desktop 設定の両方を受け取る。
- R-005: シェルは、stream のつなぎ直しの待ちと、つなぎ直しの対象にする失敗の区別を、proto の規則に従って決める。固定の間隔で繰り返すループを持たない。
- R-006: シェルが期限を指定せずに送る単発の呼び出しには、proto の規則で定めた既定の期限が当たる。
- R-007: 無音と判断する時間は proto の規則として 20 秒とし、画面とシェルは同じ値を読む。
- R-008: シェルが観測した接続の失敗は、分類を保ったまま daemon の監督へ渡り、画面から観測できる。どこからも読めない記録先には書かない。

# Assumptions

- `state_stream_silence_ms` を 30 秒から 20 秒へ下げることで、画面が stream を切ってつなぎ直す契機も 20 秒になることを受け入れている。
- シェルの単発の呼び出しの既定の期限が 30 秒から proto の値（120 秒）に変わることを受け入れている。
- `RetryBackoff::SERVICE` と proto の `connection_backoff` の値の重複は、この開発では直さない。
