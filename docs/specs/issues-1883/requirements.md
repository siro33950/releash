# Context

- 正本: [#1883 `[02] 呼び出しに期限と取り消しを通す`](https://github.com/siro33950/releash/issues/1883)
- 最初の周の調査基準は branch `feat/issues/1883` の `42be41e0`。
- アーキテクチャの正本は `AGENTS.md`。アプリケーションロジックは daemon（サーバ）が所有し、client は表示とレイアウト制御、入力受付、サーバの呼び出しと購読、表示用フォーマットだけを担う。Tauri のシェルは Rust で書かれていても client である。
- 依存する #1880（`[01] 失敗の分類をエラーの値に持たせる`）は CLOSED であり、`42be41e0` にマージ済みである。失敗の分類 `FailureKind` から Connect のエラーコードへの変換は `src-tauri/src/adaptor/protocol/connect.rs:26-47` の 1 か所にあり、`FailureKind::Expired` が `DeadlineExceeded`、`FailureKind::Cancelled` が `Canceled` に対応する。
- #1878（`[01] 状態の変化を購読で届ける土台を作り、Repository 一覧を移す`）が定めた通信の規則を前提にする。daemon が持っている状態は daemon が配信し、client は購読する。状態を変える操作と、client の入力に対する計算だけを単発の呼び出しにする。生存確認は bookmark で行う（`src-tauri/src/domain/state_subscription/mod.rs:27,94,262`）。
- #1878 の規則は導入済みだが移行は途中であり、`42be41e0` では daemon の状態を返す単発の呼び出しが残っている（`proto/client.proto:3137-3139` の `RefreshWorkspaces` / `GetWorkspaces` / `GetReviewBlob` ほか）。これらは #1885 以降で購読へ移るまでの間も単発の呼び出しであり、本 ISSUE の期限の対象になる。
- 期限の扱いは gRPC の deadline に従う（https://grpc.io/docs/guides/deadlines/ ）。client が期限を付け、server はその期限を超えて処理を続けず、期限切れは `DEADLINE_EXCEEDED`、client の中断は `CANCELLED` で終わる。
- 現行実装の確認先: `src-tauri/src/adaptor/controller/api/client.rs`、`src-tauri/src/adaptor/controller/api/client_service.rs`、`src-tauri/src/adaptor/controller/api/state_subscription.rs`、`src-tauri/src/adaptor/controller/client/dispatch.rs`、`src-tauri/src/adaptor/controller/client/repository/mod.rs`、`src-tauri/src/adaptor/gateway/desktop_client.rs`、`src-tauri/build.rs`、`src/lib/client.ts`、`proto/client.proto`
- 通信基盤は connectrpc 0.9.1（`src-tauri/Cargo.toml:35`）。server 側の期限は `DeadlinePolicy` で設定する。設定できるのは既定の期限、client 値の下限・上限、stream の item への適用の有無、無通信の上限であり、いずれも service 単位で、rpc method ごとに外すことはできない。
- `DeadlinePolicy` の既定の期限は client 値の下限・上限の調停の対象ではなく、client が期限の header を送らない呼び出しにだけ当たる。
- 期限が決まった呼び出しは handler の future ごと絶対期限で打ち切られ、`DeadlineExceeded` になる。server streaming でも handler の future（stream を返すまで）は同じ扱いで、stream を開いた後の item の流れは、stream の item への適用を有効にしない限り縛られない。
- handler は `RequestContext` を受け取り、`deadline()` で調停済みの絶対期限、`time_remaining()` で残り時間を読める。
- connectrpc は、handler が `tokio::spawn` で切り離した task の後始末を handler 側の責務としている。期限で打ち切られるのは handler の future であり、切り離した task には及ばない。
- `tokio_util`（取り消し用の token の出どころ）は `src-tauri/Cargo.lock` に 0.7.19 で入っているが、`src-tauri/Cargo.toml` の直接依存ではない。
- 同じマイルストーン（[02] UI と daemon の間の通信の仕組みを一本化する）の中で、期限を store と外部プロセスへ引き継ぐのは #1893、同時実行の枠を優先度で分けるのは #1894 が担う。

# Outcome

対象者は、Releash の UI を使う利用者と、UI と daemon の間の通信を実装・保守する開発者である。

現在、daemon は単発の呼び出しに既定の期限を持たない。client が期限を付けない呼び出しは、終わらない限り走り続ける。client が呼び出しをやめても、daemon 側で切り離された処理は止まらず、同時実行の枠（64 件）を握り続ける。枠が尽きると、以後の呼び出しは枠の不足で拒否される。期限切れや中断を処理の先へ伝える仕組みは無い。

変更後は、単発の呼び出しが必ず期限の下で動く。client が期限を付けない呼び出しには daemon の既定の期限が当たり、期限切れは `DEADLINE_EXCEEDED`、client の中断は `CANCELLED` で終わる。期限切れと中断は呼び出しの入口から処理の先へ伝わり、切り離された処理も呼び出しと同時に終わって同時実行の枠を解放する。購読の stream は開いた後は期限の対象にならず、生存は bookmark で判断する。

# Current Behavior

調査基準 `42be41e0` のコードで確認した挙動である。

## 単発の呼び出しは全て `execute` を通り、処理は切り離される

- 単発の command の rpc ハンドラは build script が生成し、いずれも `self.execute(...)` を呼ぶ（`src-tauri/build.rs:119-144`。`method.server_streaming()` の rpc は生成対象外で、`src-tauri/src/adaptor/controller/api/client_service.rs` が手書きのハンドラを追加する）。
- 生成されたハンドラと手書きのハンドラは、いずれも `connectrpc::RequestContext` を `_ctx` として受け取り、使わずに捨てている（`src-tauri/build.rs:140`、`client_service.rs:3,18,61,78,103,123,143,154,165`）。期限を読む口はあるが読んでいない。
- `execute` は同時実行の枠を取ってから、処理を `tokio::spawn` で切り離し、その完了を待つ（`src-tauri/src/adaptor/controller/api/client.rs:125-176`。切り離しは `client.rs:161`）。待っている future が drop されても、切り離した task は走り続ける。
- `StopWatching` と `WatchFiles` / `WatchGitDirectory` は、`execute` / `watch` の中で `tokio::task::spawn_blocking` を使う（`client.rs:135,193`）。`spawn_blocking` の task は外から中断できない。
- command の実処理も、多くが `spawn_blocking` 上の同期呼び出しである（`src-tauri/src/adaptor/controller/client/repository/mod.rs:1-6,35-57` ほか）。
- worktree の変更を伴う command は、`dispatch_admitted` がさらに `tokio::spawn` で切り離す（`src-tauri/src/adaptor/controller/client/dispatch.rs:135-148`）。
- 切り離した task の `JoinError` を Connect のエラーへ写す処理が、同じ形で 3 か所にある（`client.rs:139-146,168-175,197-204`）。3 か所とも `FailureKind::Internal` に分類する。
- 取り消しを受け取る仕組みは無い。`src-tauri/src` に取り消し用の token を扱うコードは無い。

## 同時実行の枠は 64 件で、切り離した処理が握り続ける

- 枠は 64 件の `Semaphore` である（`client.rs:57`）。
- 取得は待たずに試すだけで、取れなければ `CLIENT_REQUEST_LIMIT`（`FailureKind::Capacity` → `ResourceExhausted`）で拒否する（`client.rs:107-120`）。
- permit の保持は 4 通りに分かれている。`execute` の主経路は切り離した async task が持ち（`client.rs:162`）、`StopWatching` と `watch` は `spawn_blocking` の task が持ち（`client.rs:136,194`）、`GetServerInfo` と `AttachTerminalSurface` は handler の future が持つ（`client_service.rs:6,81`）。`StartStateSubscription` と `StopStateSubscription` は枠を取らない。
- client が呼び出しをやめても task は動き続けるため、枠は処理が終わるまで解放されない。`spawn_blocking` の task が持つ 2 経路は、外から中断する手段が無い。

## server 側の既定の期限が無い

- router は `DeadlinePolicy` を設定していない（`client.rs:227-240`。設定しているのは受信サイズの上限だけ）。期限の既定が無いため、client が期限を付けない呼び出しには上限が無い。
- client が期限を付けた場合は、その期限だけが効く。connectrpc 0.9.1 は `connect-timeout-ms` を読み、絶対期限で handler の future を打ち切り、超過時に `DeadlineExceeded` を返す。
- 画面の React は単発の呼び出しに既定 120 秒の期限を付ける（`src/lib/client.ts:70`）。
- Tauri のシェルは daemon への呼び出しに既定 30 秒、生存監視の `GetServerInfo` だけ 5 秒の期限を付ける（`src-tauri/src/adaptor/gateway/desktop_client.rs:19,53`）。
- したがって期限がまったく無い呼び出しは、これらの client が既定を外した場合と、ほかの呼び出し元がある場合に生じる。

## 購読の stream は期限を付けずに開かれている

- client は `SubscribePush`、`OpenStateStream`、`SubscribeTerminalSurfaces` を `timeoutMs: 0` で開く（`src/lib/client.ts:148,269,470`）。この値では期限の header を送らない。
- 3 つの stream のハンドラは、いずれも await せずに stream を返す（`client_service.rs:16-56,59-74,141-150`）。
- connectrpc 0.9.1 の `DeadlinePolicy` は、stream の item への絶対期限の適用が既定で無効であり、有効にしない限り最初の応答までしか期限が効かない。既定の期限を設定すると、client が期限を送らない stream にも、開くまでの間はその既定が当たる。
- stream が生きていることの判断は、daemon が流す bookmark と、client 側の無通信の監視（30 秒。`src/lib/client.ts:229,261`）で行っている。

# Scope / Non-goals

今回変更する対象。

- `ClientService` の単発の呼び出し（`execute` を通る command 群と、stream でない手書きハンドラ: `GetServerInfo`、`WatchFiles`、`WatchGitDirectory`、`AttachTerminalSurface`、`StartStateSubscription`、`StopStateSubscription`）への server 側の期限。
- server の既定の期限（client が期限を付けない場合に当たるもの）。
- 期限切れと client の中断を処理の先へ伝える取り消し。
- 期限切れ・中断時の、切り離した処理の終了と同時実行の枠の解放。permit の保持を全経路で揃えることを含む。
- 切り離した task の終わり方を失敗の分類へ写す処理を `client.rs` の 1 か所にまとめること。
- 期限切れ・中断のエラーコード。

今回変更しない対象。

- 購読の stream（`SubscribePush`、`OpenStateStream`、`SubscribeTerminalSurfaces`）を開いた後に期限を持たせること。開いた後の item の流れは期限の対象外にする。
- client が指定した期限に対する daemon 側の上限・下限。client の指定はそのまま使う。
- 画面の React が付ける既定の期限（`src/lib/client.ts:70` の 120 秒）。
- Tauri のシェルが付ける既定の期限（`src-tauri/src/adaptor/gateway/desktop_client.rs:19` の 30 秒、生存監視の 5 秒）。
- 既に同期処理として動き始めた処理の内側で取り消しを受け取れるようにすること。`spawn_blocking` 上の同期処理と、その先の store・外部プロセスへの引き継ぎは #1893 が担う。
- 期限を store と外部プロセスへ引き継ぐこと（#1893）。
- 同時実行の枠を優先度で分けること、枠の数の見直し（#1894）。
- 呼び出しが終わった後も続く同期処理の資源上限と、上限に達したときの振る舞い。同時実行の枠は呼び出しの終了と同時に解放されるため（R-004）、開始済みの同期処理を縛る上限は今回置かない（#1894）。
- CLI / hook 用の HTTP local API（`src-tauri/src/adaptor/controller/api/workflow.rs` ほか）への期限と取り消し。
- 失敗の分類の仕組みそのもの（#1880 で完了）。
- client 側の接続状態の保持、つなぎ直し、再接続時の画面の扱い（#1891、#1895、#1896）。

# Requirements

- R-001: client が期限を付けない単発の呼び出しには、daemon の既定の期限 120 秒が適用され、その期限を超えた呼び出しは `DEADLINE_EXCEEDED` で終わる。
- R-002: client が期限を付けた単発の呼び出しは、その期限を超えると `DEADLINE_EXCEEDED` で終わる。client が指定した期限は daemon 側で変更されない。
- R-003: client が単発の呼び出しをやめたとき、その呼び出しは `CANCELLED` として終わる。
- R-004: 期限切れまたは client の中断で終わった単発の呼び出しでは、daemon 側で切り離された処理も同時に終わり、その呼び出しが確保していた同時実行の枠が解放される。
- R-005: 期限切れと client の中断は、呼び出しの入口から処理の先へ伝わり、取り消しを受け取った処理は実行を止める。
- R-006: 購読の stream を開いた後は期限の対象にならず、単発の呼び出しの期限を超えて何も流れていない間も、期限切れで打ち切られない。stream を開く呼び出し自体は、単発の呼び出しと同じ期限の対象になる。stream が生きていることの判断は bookmark で行う。

# Assumptions / Open Questions

なし。
